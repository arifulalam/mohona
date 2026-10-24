use std::path::PathBuf;
use std::sync::Arc;

use mohona_manager::{AddRequest, AddResult, DuplicatePolicy, Manager, ManagerError};
use mohona_net::RequestContext;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use crate::info::DaemonInfo;
use crate::protocol::{Call, DownloadInfo, Request, Response};

/// The longest request line accepted.
const MAX_LINE: usize = 1024 * 1024;

pub struct ServerConfig {
    /// Where to write [`DaemonInfo`] (removed again on exit).
    pub info_path: PathBuf,
    /// Where downloads go unless the client says otherwise.
    pub default_dir: PathBuf,
    /// Pass a [`Notify`] to stop the daemon from outside (Ctrl-C); a
    /// `shutdown` call stops it too.
    pub shutdown: Arc<Notify>,
}

struct State {
    manager: Manager,
    token: String,
    default_dir: PathBuf,
    shutdown: Arc<Notify>,
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
        Call::List => match manager.list().await {
            Ok(rows) => Response::Downloads {
                downloads: rows
                    .into_iter()
                    .map(|r| DownloadInfo {
                        id: r.id,
                        url: r.url,
                        filename: r.filename,
                        status: r.status.as_str().to_string(),
                        downloaded: r.downloaded,
                        size: r.size,
                        error: r.error,
                        path: r.path,
                    })
                    .collect(),
            },
            Err(e) => error(e),
        },
        Call::Pause { id } => done(manager.pause(id).await),
        Call::Resume { id } => done(manager.resume(id).await),
        Call::Cancel { id } => done(manager.cancel(id).await),
        Call::Remove { id, delete_file } => done(manager.remove(id, delete_file).await),
        Call::Shutdown => Response::Done,
    }
}
