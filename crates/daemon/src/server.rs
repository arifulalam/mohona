use std::path::PathBuf;
use std::sync::Arc;

use mohona_manager::{AddRequest, AddResult, DuplicatePolicy, Manager, ManagerError, SpeedTarget};
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
            let mut request = AddRequest::new(url, folder_or_default(state, dest_dir).await);
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
            let folder = folder_or_default(state, dest_dir).await;
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
            let folder = folder_or_default(state, dest_dir).await;
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
        Call::GetConfig => config(state).await,
        Call::SetConfig {
            download_folder,
            speed_limit,
            max_active,
            notifications,
            keep_in_tray,
        } => {
            let change = ConfigChange {
                download_folder,
                speed_limit,
                max_active,
                notifications,
                keep_in_tray,
            };
            set_config(state, change).await
        }
        Call::Pause { id } => done(manager.pause(id).await),
        Call::Resume { id } => done(manager.resume(id).await),
        Call::Cancel { id } => done(manager.cancel(id).await),
        Call::Remove { id, delete_file } => done(manager.remove(id, delete_file).await),
        Call::Shutdown => Response::Done,
    }
}

/// The settings key of the folder downloads go to.
const FOLDER_KEY: &str = "download_folder";
/// The settings key of the global speed limit (the manager's own).
const SPEED_KEY: &str = "speed_limit";
const NOTIFY_KEY: &str = "notifications";
const TRAY_KEY: &str = "keep_in_tray";

/// The folder a client named, else the folder the person chose, else the default.
async fn folder_or_default(state: &State, named: Option<String>) -> PathBuf {
    if let Some(named) = named {
        return PathBuf::from(named);
    }
    match state.manager.setting(FOLDER_KEY).await {
        Ok(Some(chosen)) if !chosen.is_empty() => PathBuf::from(chosen),
        _ => state.default_dir.clone(),
    }
}

/// The yes/no settings are stored as "1" and "0"; both are on until switched off.
async fn flag(state: &State, key: &str) -> bool {
    !matches!(state.manager.setting(key).await, Ok(Some(v)) if v == "0")
}

async fn config(state: &State) -> Response {
    let manager = &state.manager;
    let folder = folder_or_default(state, None).await;
    let speed_limit = manager
        .setting(SPEED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0);
    let max_active = match manager.queues().await {
        Ok(queues) => queues.first().map_or(4, |q| q.max_active),
        Err(e) => return error(e),
    };
    Response::Config {
        download_folder: folder.display().to_string(),
        default_folder: state.default_dir.display().to_string(),
        speed_limit,
        max_active,
        notifications: flag(state, NOTIFY_KEY).await,
        keep_in_tray: flag(state, TRAY_KEY).await,
    }
}

struct ConfigChange {
    download_folder: Option<String>,
    speed_limit: Option<u64>,
    max_active: Option<u32>,
    notifications: Option<bool>,
    keep_in_tray: Option<bool>,
}

async fn set_config(state: &State, change: ConfigChange) -> Response {
    let ConfigChange {
        download_folder: folder,
        speed_limit,
        max_active,
        notifications,
        keep_in_tray,
    } = change;
    let manager = &state.manager;
    // Check everything first, so a refused value changes nothing.
    let folder = match folder.map(|f| f.trim().to_string()) {
        None => None,
        Some(f) if f.is_empty() => Some(String::new()),
        Some(f) => {
            let path = PathBuf::from(&f);
            if !path.is_absolute() || f.chars().any(char::is_control) {
                return error(
                    "the download folder must be a full path such as C:\\Users\\you\\Downloads",
                );
            }
            if let Err(e) = std::fs::create_dir_all(&path) {
                return error(format!("cannot use the folder {f}: {e}"));
            }
            Some(f)
        }
    };
    if max_active.is_some_and(|n| !(1..=32).contains(&n)) {
        return error("simultaneous downloads must be between 1 and 32");
    }
    if let Some(f) = folder {
        let result = if f.is_empty() {
            manager.set_setting(FOLDER_KEY, "").await
        } else {
            manager.set_setting(FOLDER_KEY, &f).await
        };
        if let Err(e) = result {
            return error(e);
        }
    }
    if let Some(limit) = speed_limit {
        let value = (limit > 0).then_some(limit);
        if let Err(e) = manager.set_speed_limit(SpeedTarget::Global, value).await {
            return error(e);
        }
    }
    for (key, value) in [(NOTIFY_KEY, notifications), (TRAY_KEY, keep_in_tray)] {
        if let Some(on) = value {
            if let Err(e) = manager.set_setting(key, if on { "1" } else { "0" }).await {
                return error(e);
            }
        }
    }
    if let Some(n) = max_active {
        match manager.queues().await {
            Ok(queues) => {
                for q in queues.iter().take(1) {
                    if let Err(e) = manager.set_queue_limit(q.id, n).await {
                        return error(e);
                    }
                }
            }
            Err(e) => return error(e),
        }
    }
    config(state).await
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
