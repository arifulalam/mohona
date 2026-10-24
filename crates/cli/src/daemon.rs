//! `mohona daemon`: the background engine that the browser extension and
//! other programs talk to.

use std::path::PathBuf;
use std::sync::Arc;

use mohona_daemon::{default_info_path, serve, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use tokio::sync::Notify;

use crate::describe;

/// The folder downloads go to when a client does not name one.
fn default_folder() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("Downloads")
}

pub async fn run(
    db: PathBuf,
    info: Option<PathBuf>,
    folder: Option<PathBuf>,
) -> Result<(), String> {
    let store = Store::open(&db).map_err(|e| format!("cannot open {}: {e}", db.display()))?;
    let client = HttpClient::new(&ClientConfig::default()).map_err(|e| describe(&e))?;
    let manager =
        Manager::start(store, client, ManagerConfig::default()).map_err(|e| e.to_string())?;
    let info_path = info.unwrap_or_else(default_info_path);
    let shutdown = Arc::new(Notify::new());
    let stop = shutdown.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        stop.notify_one();
    });
    eprintln!(
        "Mohona is running. Press Ctrl-C to stop it. (Details for other programs: {})",
        info_path.display()
    );
    serve(
        manager,
        ServerConfig {
            info_path,
            default_dir: folder.unwrap_or_else(default_folder),
            shutdown,
        },
    )
    .await
    .map_err(|e| e.to_string())
}
