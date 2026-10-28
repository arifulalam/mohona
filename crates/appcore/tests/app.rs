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

#[tokio::test]
async fn settings_pages_round_trip_through_the_engine() {
    use mohona_appcore::{ConfigChange, ExtensionSettings};
    let dir = tempfile::tempdir().unwrap();
    let (engine, shutdown) = engine(dir.path()).await;

    let folder = dir.path().join("chosen");
    let config = engine
        .set_config(ConfigChange {
            download_folder: Some(folder.display().to_string()),
            speed_limit: Some(250_000),
            max_active: Some(3),
            ..ConfigChange::default()
        })
        .await
        .unwrap();
    assert_eq!(config.speed_limit, Some(250_000));
    assert_eq!(config.max_active, 3);
    assert_eq!(engine.config().await.unwrap(), config);
    let refused = engine
        .set_config(ConfigChange {
            max_active: Some(0),
            ..ConfigChange::default()
        })
        .await;
    assert!(refused.is_err());
    assert_eq!(engine.config().await.unwrap().max_active, 3);

    // The extension's settings: defaults first, then a change the browsers will see as newer.
    let mut ext = engine.extension_settings().await.unwrap();
    assert_eq!(ext, ExtensionSettings::default());
    ext.excluded_hosts = vec!["bank.example".into()];
    ext.min_size_bytes = 5 << 20;
    let saved = engine.set_extension_settings(ext.clone()).await.unwrap();
    assert_eq!(saved, ext);
    assert_eq!(engine.extension_settings().await.unwrap(), ext);
    shutdown.notify_one();
}

#[tokio::test]
async fn open_and_show_only_work_for_finished_files_that_still_exist() {
    let dir = tempfile::tempdir().unwrap();
    let server = files().await;
    let (engine, shutdown) = engine(dir.path()).await;
    let id = engine
        .add(&format!("http://{server}/file?size=20000&seed=4"), None)
        .await
        .unwrap();
    let started = Instant::now();
    let mut tracker = SpeedTracker::new();
    loop {
        let rows = engine.list(&mut tracker).await.unwrap();
        if rows.iter().any(|r| r.id == id && r.status == "completed") {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let path = engine.finished_file(id).await.unwrap();
    assert!(path.is_file());
    assert!(matches!(
        engine.finished_file(12345).await,
        Err(AppError::Input(_))
    ));
    std::fs::remove_file(&path).unwrap();
    let err = engine.finished_file(id).await.unwrap_err();
    assert!(err.to_string().contains("no longer there"), "{err}");
    shutdown.notify_one();
}

#[tokio::test]
async fn the_switches_for_messages_and_the_tray_are_kept() {
    use mohona_appcore::ConfigChange;
    let dir = tempfile::tempdir().unwrap();
    let (engine, shutdown) = engine(dir.path()).await;
    let c = engine.config().await.unwrap();
    assert!(c.notifications && c.keep_in_tray);
    let c = engine
        .set_config(ConfigChange {
            keep_in_tray: Some(false),
            ..ConfigChange::default()
        })
        .await
        .unwrap();
    assert!(c.notifications && !c.keep_in_tray);
    shutdown.notify_one();
}

#[tokio::test]
async fn the_schedule_and_the_end_action_go_through_the_app() {
    use mohona_appcore::PlanWindow;
    let dir = tempfile::tempdir().unwrap();
    let (engine, shutdown) = engine(dir.path()).await;
    let plan = engine.plan().await.unwrap();
    assert!(plan.windows.is_empty() && plan.open_now && plan.action.is_none());

    let night = PlanWindow {
        days: vec!["mon".into(), "tue".into()],
        start: "23:00".into(),
        end: "06:00".into(),
        speed_limit: None,
    };
    let plan = engine.set_plan(vec![night.clone()]).await.unwrap();
    assert_eq!(plan.windows, vec![night]);
    let refused = engine
        .set_plan(vec![PlanWindow {
            days: vec![],
            start: "10:00".into(),
            end: "10:00".into(),
            speed_limit: None,
        }])
        .await;
    assert!(refused.is_err(), "a window that starts and ends together");
    assert_eq!(engine.plan().await.unwrap().windows.len(), 1);
    assert!(engine.set_plan(vec![]).await.unwrap().windows.is_empty());

    let plan = engine
        .set_after_action(Some("sleep".into()), Some(600))
        .await
        .unwrap();
    assert_eq!(plan.action.as_deref(), Some("sleep"));
    assert!(engine
        .set_after_action(Some("run:calc".into()), None)
        .await
        .is_err());
    assert_eq!(engine.cancel_action().await.unwrap().action, None);
    shutdown.notify_one();
}

#[tokio::test]
async fn categories_can_be_listed_added_and_removed_through_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, shutdown) = engine(dir.path()).await;
    let first = engine.categories().await.unwrap();
    assert!(first.iter().any(|c| c.name == "Video"));
    let after = engine
        .add_category("Photos", vec!["JPG".into(), ".png".into()], None)
        .await
        .unwrap();
    let photos = after.iter().find(|c| c.name == "Photos").unwrap();
    assert_eq!(photos.extensions, ["jpg", "png"]);
    let taken = engine
        .add_category("More photos", vec!["png".into()], None)
        .await
        .unwrap_err();
    assert!(
        taken.to_string().contains("already belongs to Photos"),
        "{taken}"
    );
    let id = photos.id;
    assert!(!engine
        .remove_category(id)
        .await
        .unwrap()
        .iter()
        .any(|c| c.name == "Photos"));
    shutdown.notify_one();
}
