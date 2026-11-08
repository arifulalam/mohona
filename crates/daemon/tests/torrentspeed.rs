//! The speed limit for torrents: kept, read back, refused when absurd.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::{serve, Call, Client, DaemonInfo, Response, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use tokio::sync::Notify;

async fn start(dir: &Path) -> (Client, Arc<Notify>) {
    let store = Store::open(&dir.join("test.db")).unwrap();
    let http = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    let manager = Manager::start(store, http, ManagerConfig::default()).unwrap();
    let info_path = dir.join("daemon.json");
    let shutdown = Arc::new(Notify::new());
    tokio::spawn(serve(
        manager,
        ServerConfig {
            info_path: info_path.clone(),
            default_dir: dir.join("dl"),
            stream_program: None,
            shutdown: shutdown.clone(),
        },
    ));
    let started = Instant::now();
    loop {
        if let Ok(info) = DaemonInfo::read(&info_path) {
            if info.pid == std::process::id() {
                return (Client::new(&info), shutdown);
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "daemon did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_torrent_speed_limit_is_kept_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let (client, shutdown) = start(dir.path()).await;
    let read = |r: Response| match r {
        Response::TorrentSpeed { down_kb, up_kb } => (down_kb, up_kb),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        read(client.call(Call::GetTorrentSpeed).await.unwrap()),
        (None, None)
    );
    let set = |down_kb, up_kb| Call::SetTorrentSpeed { down_kb, up_kb };
    assert_eq!(
        read(client.call(set(Some(500), Some(100))).await.unwrap()),
        (Some(500), Some(100))
    );
    assert_eq!(
        read(client.call(Call::GetTorrentSpeed).await.unwrap()),
        (Some(500), Some(100))
    );
    // 0 and nothing both mean no limit.
    assert_eq!(
        read(client.call(set(Some(0), None)).await.unwrap()),
        (None, None)
    );
    assert!(matches!(
        client.call(set(Some(4_000_001), None)).await.unwrap(),
        Response::Error { .. }
    ));
    // Nothing is being shared, so there is nothing to stop.
    assert!(matches!(
        client.call(Call::GetSharing).await.unwrap(),
        Response::Sharing { items } if items.is_empty()
    ));
    assert!(matches!(
        client.call(Call::StopSharing { id: 1 }).await.unwrap(),
        Response::Error { .. }
    ));
    shutdown.notify_one();
}
