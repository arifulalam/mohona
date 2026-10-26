use std::path::PathBuf;
use std::sync::Arc;

use mohona_manager::{AddRequest, AddResult, DuplicatePolicy, Manager, ManagerError};
use mohona_net::RequestContext;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use crate::info::DaemonInfo;
use crate::protocol::{Call, DownloadInfo, Request, Response};
use crate::streams::StreamJob;

/// The longest request line accepted.
const MAX_LINE: usize = 1024 * 1024;

pub struct ServerConfig {
    /// Where to write [`DaemonInfo`] (removed again on exit).
    pub info_path: PathBuf,
    /// Where downloads go unless the client says otherwise.
    pub default_dir: PathBuf,
    /// The `mohona` program that runs stream downloads (`mohona stream`).
    /// `None`: stream calls are refused.
    pub stream_program: Option<PathBuf>,
    /// Pass a [`Notify`] to stop the daemon from outside (Ctrl-C); a
    /// `shutdown` call stops it too.
    pub shutdown: Arc<Notify>,
}

struct State {
    manager: Manager,
    token: String,
    default_dir: PathBuf,
    shutdown: Arc<Notify>,
    streams: crate::streams::Streams,
    receiver: crate::receive::Receiver,
}

/// Runs the API until shutdown, then stops the manager and removes the info
/// file. Refuses to start when another daemon answers at the address in the
/// existing info file.
pub async fn serve(manager: Manager, config: ServerConfig) -> std::io::Result<()> {
    if let Ok(old) = DaemonInfo::read(&config.info_path) {
        let alive = crate::Client::new(&old).call(Call::Ping).await.is_ok();
        if alive {
            return Err(std::io::Error::other(format!(
                "a Mohona daemon is already running (process {})",
                old.pid
            )));
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let info = DaemonInfo {
        port: listener.local_addr()?.port(),
        token: DaemonInfo::new_token(),
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    info.write(&config.info_path)?;
    let state = Arc::new(State {
        manager,
        token: info.token.clone(),
        default_dir: config.default_dir,
        shutdown: config.shutdown.clone(),
        streams: crate::streams::Streams::new(config.stream_program),
        receiver: crate::receive::Receiver::default(),
    });
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { continue };
                if !peer.ip().is_loopback() {
                    continue;
                }
                let state = Arc::clone(&state);
                tokio::spawn(async move { let _ = connection(stream, state).await; });
            }
            () = config.shutdown.notified() => break,
        }
    }
    let _ = std::fs::remove_file(&config.info_path);
    state.manager.shutdown().await;
    Ok(())
}

/// Same answer in the same time whether the token is nearly right or not.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

async fn connection(stream: TcpStream, state: Arc<State>) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read).take(MAX_LINE as u64 + 1);
    loop {
        let mut line = String::new();
        reader.set_limit(MAX_LINE as u64 + 1);
        if reader.read_line(&mut line).await? == 0 || line.len() > MAX_LINE {
            return Ok(());
        }
        // Anything that is not a proper request (a web page speaking HTTP to
        // this port, for one) gets nothing and the connection is closed.
        let Ok(request) = serde_json::from_str::<Request>(&line) else {
            return Ok(());
        };
        if !same_token(&request.token, &state.token) {
            return Ok(());
        }
        let stop = request.call == Call::Shutdown;
        let response = handle(&state, request.call).await;
        let mut text = serde_json::to_string(&response).map_err(std::io::Error::other)?;
        text.push('\n');
        write.write_all(text.as_bytes()).await?;
        if stop {
            state.shutdown.notify_one();
            return Ok(());
        }
    }
}

fn error(e: impl std::fmt::Display) -> Response {
    Response::Error {
        message: e.to_string(),
    }
}

fn done(result: Result<(), ManagerError>) -> Response {
    match result {
        Ok(()) => Response::Done,
        Err(e) => error(e),
    }
}

async fn handle(state: &State, call: Call) -> Response {
    let manager = &state.manager;
    match call {
        Call::Ping => Response::Pong {
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
        },
        Call::Add {
            url,
            dest_dir,
            cookies,
            referrer,
            user_agent,
            headers,
            sort,
            duplicates,
        } => {
            let policy = match duplicates.as_deref() {
                None | Some("ask") => DuplicatePolicy::Ask,
                Some("skip") => DuplicatePolicy::Skip,
                Some("rename") => DuplicatePolicy::Rename,
                Some(other) => {
                    return error(format!(
                        "\"{other}\" is not a duplicate policy (use ask, skip or rename)"
                    ))
                }
            };
            let mut request = AddRequest::new(
                url,
                dest_dir.map_or_else(|| state.default_dir.clone(), PathBuf::from),
            );
            request.sort_into_categories = sort;
            request.duplicates = policy;
            // Only what a browser session legitimately carries: nothing that
            // weakens certificate checks or names proxies or key files.
            request.context = RequestContext {
                cookies,
                referrer,
                user_agent,
                headers,
                ..RequestContext::default()
            };
            match manager.add_checked(request).await {
                Ok(AddResult::Added(id)) => Response::Added { id },
                Ok(AddResult::Skipped(why)) => Response::Skipped {
                    reason: why.to_string(),
                },
                Err(e) => error(e),
            }
        }
        Call::Stream {
            url,
            dest_dir,
            cookies,
            referrer,
            user_agent,
            headers,
            name,
        } => {
            let folder = dest_dir.map_or_else(|| state.default_dir.clone(), PathBuf::from);
            let shown = folder.display().to_string();
            match state.streams.start(StreamJob {
                url,
                folder,
                cookies,
                referrer,
                user_agent,
                headers,
                name,
            }) {
                Ok(job) => Response::Started { job, folder: shown },
                Err(message) => Response::Error { message },
            }
        }
        Call::ReceiveStart {
            name,
            mime,
            dest_dir,
        } => {
            let folder = dest_dir.map_or_else(|| state.default_dir.clone(), PathBuf::from);
            match state.receiver.start(&folder, &name, mime) {
                Ok(transfer) => Response::Receiving { transfer },
                Err(message) => Response::Error { message },
            }
        }
        Call::ReceivePiece {
            transfer,
            index,
            data,
        } => match state.receiver.piece(transfer, index, &data) {
            Ok(()) => Response::Done,
            Err(message) => Response::Error { message },
        },
        Call::ReceiveEnd { transfer } => match state.receiver.finish(transfer) {
            Ok(path) => Response::Saved {
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path: path.display().to_string(),
            },
            Err(message) => Response::Error { message },
        },
        Call::ReceiveAbort { transfer } => {
            state.receiver.abort(transfer);
            Response::Done
        }
        Call::Jobs => Response::Jobs {
            jobs: state.streams.list(),
        },
        Call::SettingGet { key } => match setting_key(&key) {
            Err(message) => Response::Error { message },
            Ok(()) => match manager.setting(&key).await {
                Ok(value) => Response::Setting { value },
                Err(e) => error(e),
            },
        },
        Call::SettingSet { key, value } => match setting_key(&key) {
            Err(message) => Response::Error { message },
            Ok(()) if value.len() > MAX_SETTING => Response::Error {
                message: "the setting is too large".into(),
            },
            Ok(()) => done(manager.set_setting(&key, &value).await),
        },
        Call::List => match manager.list().await {
            Ok(rows) => {
                let mut downloads = Vec::with_capacity(rows.len());
                for r in rows {
                    let expired = r.status == mohona_store::Status::Failed
                        && r.error.as_deref().is_some_and(|e| {
                            e.contains("answered 403") || e.contains("answered 410")
                        });
                    let page = if expired {
                        manager
                            .request_context(r.id)
                            .await
                            .ok()
                            .and_then(|c| c.referrer)
                    } else {
                        None
                    };
                    downloads.push(DownloadInfo {
                        id: r.id,
                        url: r.url,
                        filename: r.filename,
                        status: r.status.as_str().to_string(),
                        downloaded: r.downloaded,
                        size: r.size,
                        error: r.error,
                        path: r.path,
                        page,
                        expired,
                    });
                }
                Response::Downloads { downloads }
            }
            Err(e) => error(e),
        },
        Call::Refresh {
            id,
            url,
            cookies,
            referrer,
            user_agent,
        } => done(
            manager
                .refresh_link(
                    id,
                    &url,
                    RequestContext {
                        cookies,
                        referrer,
                        user_agent,
                        ..RequestContext::default()
                    },
                )
                .await,
        ),
        Call::Pause { id } => done(manager.pause(id).await),
        Call::Resume { id } => done(manager.resume(id).await),
        Call::Cancel { id } => done(manager.cancel(id).await),
        Call::Remove { id, delete_file } => done(manager.remove(id, delete_file).await),
        Call::Shutdown => Response::Done,
    }
}

/// The largest setting value the daemon keeps for a client.
const MAX_SETTING: usize = 64 * 1024;

/// Clients may only touch `extension.*` settings (never the engine's own).
fn setting_key(key: &str) -> Result<(), String> {
    let ok = key.len() <= 64
        && key.starts_with("extension.")
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(format!("\"{key}\" is not a setting clients may use"))
    }
}
