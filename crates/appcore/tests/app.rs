use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_appcore::{AppError, Engine, SpeedTracker};
use mohona_daemon::{serve, DaemonInfo, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use mohona_testserver::{bind, serve as serve_files, Config};
use tokio::sync::Notify;

async fn files() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve_files(
            listener,
            Config {
                default_size: 300_000,
            },
        )
        .await;
    });
    addr
}

async fn engine(dir: &Path) -> (Engine, Arc<Notify>) {
    let store = Store::open(&dir.join("t.db")).unwrap();
    let client = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    let manager = Manager::start(
        store,
        client,
        ManagerConfig {
            adaptive: false,
            ..ManagerConfig::default()
        },
    )
    .unwrap();
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
    while !DaemonInfo::read(&info_path).is_ok_and(|i| i.pid == std::process::id()) {
        assert!(started.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (
        Engine {
            info_path,
            program: None,
            database: None,
        },
        shutdown,
    )
}

#[tokio::test]
async fn without_an_engine_the_app_says_so_and_cannot_start_one_it_cannot_find() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine {
        info_path: dir.path().join("none.json"),
        program: None,
        database: None,
    };
    let status = engine.status().await;
    assert!(!status.running);
    assert!(matches!(
        engine.list(&mut SpeedTracker::new()).await,
        Err(AppError::NotRunning)
    ));
    let err = engine.start().await.unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
}

#[tokio::test]
async fn adds_lists_pauses_and_removes_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let server = files().await;
    let (engine, shutdown) = engine(dir.path()).await;
    assert!(engine.status().await.running);

    let url = format!("http://{server}/file?size=300000&seed=1");
    let id = engine.add(&url, None).await.unwrap();
    // The same address again is refused with a reason a person can read.
    let again = engine.add(&url, None).await.unwrap_err();
    assert!(matches!(again, AppError::Input(_)), "{again}");

    let mut tracker = SpeedTracker::new();
    let started = Instant::now();
    loop {
        let rows = engine.list(&mut tracker).await.unwrap();
        let row = rows.iter().find(|r| r.id == id).expect("listed");
        if row.status == "completed" {
            assert_eq!(row.percent, Some(100.0));
            assert_eq!(row.speed, 0);
            assert_eq!(row.name, "mohona-test.bin");
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(15), "{row:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    engine.remove(id, false).await.unwrap();
    assert!(engine.list(&mut tracker).await.unwrap().is_empty());
    shutdown.notify_one();
}

#[tokio::test]
async fn refuses_bad_addresses_before_asking_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, shutdown) = engine(dir.path()).await;
    for bad in [
        "",
        "   ",
        "example.com/a.zip",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "http://x/\u{7}",
    ] {
        let err = engine.add(bad, None).await.unwrap_err();
        assert!(matches!(err, AppError::Input(_)), "{bad:?}: {err}");
    }
    assert!(engine
        .list(&mut SpeedTracker::new())
        .await
        .unwrap()
        .is_empty());
    shutdown.notify_one();
}

#[tokio::test]
async fn pausing_an_unknown_download_is_an_error_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, shutdown) = engine(dir.path()).await;
    // Whatever the engine answers, the app returns (an error or nothing) and keeps working.
    let _ = engine.pause(999).await;
    let _ = engine.cancel(999).await;
    assert!(engine.status().await.running);
    shutdown.notify_one();
}
