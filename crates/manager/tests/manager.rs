//! The manager against the local test server.

use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use mohona_core::RetryPolicy;
use mohona_manager::{DownloadStatus as Status, Manager, ManagerConfig, ManagerEvent, SpeedTarget};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use mohona_testserver::{bind, content_byte, serve, Config};

async fn start_server() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve(
            listener,
            Config {
                default_size: 1_000_000,
            },
        )
        .await;
    });
    addr
}

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap()
}

fn config() -> ManagerConfig {
    ManagerConfig {
        adaptive: false,
        connections: 1,
        // Failing requests give up fast; the manager's own retries are what
        // these tests look at.
        retry: RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
            max_attempts: 1,
            max_throttles: 1,
            ..RetryPolicy::default()
        },
        max_retries: 3,
        retry_delay: Duration::from_millis(100),
    }
}

fn manager_at(db: &Path) -> Manager {
    Manager::start(Store::open(db).unwrap(), client(), config()).unwrap()
}

async fn wait_for(manager: &Manager, id: i64, status: Status) {
    let start = Instant::now();
    loop {
        let rows = manager.list().await.unwrap();
        let row = rows.iter().find(|r| r.id == id).expect("download exists");
        if row.status == status {
            return;
        }
        // Generous: on Windows a refused connection takes about 2 s to
        // report, and the retry tests make several of them.
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "download {id} is {:?}, expected {status:?}",
            row.status
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_bytes(manager: &Manager, id: i64, at_least: u64) {
    let start = Instant::now();
    loop {
        let rows = manager.list().await.unwrap();
        let row = rows.iter().find(|r| r.id == id).unwrap();
        if row.downloaded >= at_least {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "no progress");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn check_content(path: &Path, len: usize) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len(), len);
    for (i, b) in data.iter().enumerate() {
        assert_eq!(*b, content_byte(i as u64), "byte {i}");
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn a_download_runs_and_is_saved_as_completed() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let mut events = manager.subscribe();
    let out = dir.path().join("out");

    let id = manager
        .add(format!("http://{addr}/file?size=300000"), &out)
        .await
        .unwrap();
    wait_for(&manager, id, Status::Completed).await;

    let row = manager.list().await.unwrap().remove(0);
    assert_eq!(row.size, Some(300_000));
    assert_eq!(row.downloaded, 300_000);
    assert!(row.finished_at.is_some());
    check_content(Path::new(row.path.as_deref().unwrap()), 300_000);

    let mut seen = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let ManagerEvent::State { status, .. } = e {
            seen.push(status);
        }
    }
    assert_eq!(
        seen,
        vec![Status::Probing, Status::Downloading, Status::Completed]
    );
}

#[tokio::test]
async fn a_failed_download_keeps_its_error_and_can_be_retried() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let id = manager
        .add(format!("http://{addr}/missing"), dir.path().join("out"))
        .await
        .unwrap();
    wait_for(&manager, id, Status::Failed).await;
    let row = manager.list().await.unwrap().remove(0);
    assert!(row.error.unwrap().contains("404"));

    manager.resume(id).await.unwrap();
    wait_for(&manager, id, Status::Failed).await;
}

#[tokio::test]
async fn a_queue_never_runs_more_than_its_limit() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    manager.set_queue_limit(1, 2).await.unwrap();
    let out = dir.path().join("out");
    let mut ids = Vec::new();
    for i in 0..5 {
        // About half a second each at 400 KB/s.
        let url = format!("http://{addr}/file?size=200000&limit=400000&n={i}");
        ids.push(manager.add(url, &out).await.unwrap());
    }
    let mut most = 0;
    let start = Instant::now();
    loop {
        let rows = manager.list().await.unwrap();
        let active = rows.iter().filter(|r| r.status.is_active()).count();
        most = most.max(active);
        if rows.iter().all(|r| r.status == Status::Completed) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{:?}",
            rows.iter()
                .map(|r| (r.id, r.status, r.downloaded, r.error.clone()))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(most, 2, "should use both slots but never a third");
}

#[tokio::test]
async fn queued_downloads_start_in_order() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    manager.set_queue_limit(1, 1).await.unwrap();
    let mut events = manager.subscribe();
    let out = dir.path().join("out");
    let mut ids = Vec::new();
    for i in 0..3 {
        let url = format!("http://{addr}/file?size=50000&n={i}");
        ids.push(manager.add(url, &out).await.unwrap());
    }
    wait_for(&manager, ids[2], Status::Completed).await;
    let mut started = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let ManagerEvent::State {
            id,
            status: Status::Probing,
        } = e
        {
            started.push(id);
        }
    }
    assert_eq!(started, ids);
}

#[tokio::test]
async fn pause_keeps_the_data_and_resume_finishes_the_file() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let out = dir.path().join("out");
    let id = manager
        .add(
            format!("http://{addr}/file?size=2000000&limit=1000000"),
            &out,
        )
        .await
        .unwrap();
    wait_for_bytes(&manager, id, 300_000).await;

    manager.pause(id).await.unwrap();
    wait_for(&manager, id, Status::Paused).await;
    let paused = manager.list().await.unwrap().remove(0);
    assert!(paused.downloaded > 0 && paused.downloaded < 2_000_000);
    assert!(names(&out).iter().any(|n| n.ends_with(".part")));

    manager.resume(id).await.unwrap();
    wait_for(&manager, id, Status::Completed).await;
    let row = manager.list().await.unwrap().remove(0);
    check_content(Path::new(row.path.as_deref().unwrap()), 2_000_000);
    assert_eq!(
        names(&out).len(),
        1,
        "no partial files left: {:?}",
        names(&out)
    );
}

#[tokio::test]
async fn cancel_removes_partial_data() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let out = dir.path().join("out");
    let id = manager
        .add(
            format!("http://{addr}/file?size=2000000&limit=1000000"),
            &out,
        )
        .await
        .unwrap();
    wait_for_bytes(&manager, id, 200_000).await;
    manager.cancel(id).await.unwrap();
    wait_for(&manager, id, Status::Canceled).await;
    assert!(names(&out).is_empty(), "left behind: {:?}", names(&out));
}

#[tokio::test]
async fn pausing_a_queued_download_stops_it_from_starting() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    manager.set_queue_limit(1, 1).await.unwrap();
    let out = dir.path().join("out");
    let first = manager
        .add(format!("http://{addr}/file?size=400000&limit=400000"), &out)
        .await
        .unwrap();
    let second = manager
        .add(format!("http://{addr}/file?size=1000&n=2"), &out)
        .await
        .unwrap();
    manager.pause(second).await.unwrap();
    wait_for(&manager, first, Status::Completed).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rows = manager.list().await.unwrap();
    assert_eq!(rows[1].status, Status::Paused);
}

#[tokio::test]
async fn shutdown_and_restart_continue_where_they_stopped() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let out = dir.path().join("out");
    let manager = manager_at(&db);
    let id = manager
        .add(
            format!("http://{addr}/file?size=2000000&limit=1000000"),
            &out,
        )
        .await
        .unwrap();
    wait_for_bytes(&manager, id, 300_000).await;
    manager.shutdown().await;

    // Between runs the download is waiting, with its partial file kept.
    let store = Store::open(&db).unwrap();
    let row = store.download(id).unwrap().unwrap();
    assert_eq!(row.status, Status::Queued, "{row:?}");
    assert!(names(&out).iter().any(|n| n.ends_with(".part")));
    drop(store);

    let manager = manager_at(&db);
    wait_for(&manager, id, Status::Completed).await;
    let row = manager.list().await.unwrap().remove(0);
    check_content(Path::new(row.path.as_deref().unwrap()), 2_000_000);
}

#[tokio::test]
async fn a_crash_leaves_downloads_that_start_again() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let out = dir.path().join("out");
    // Pretend the app died while this was downloading.
    {
        let store = Store::open(&db).unwrap();
        let url = format!("http://{addr}/file?size=100000");
        let id = store
            .add_download(&url, &out.to_string_lossy(), None)
            .unwrap();
        store
            .set_status(id, mohona_store::Status::Downloading, None)
            .unwrap();
    }
    let manager = manager_at(&db);
    let id = manager.list().await.unwrap()[0].id;
    wait_for(&manager, id, Status::Completed).await;
}

#[tokio::test]
async fn remove_deletes_the_row_and_optionally_the_file() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let out = dir.path().join("out");
    let id = manager
        .add(format!("http://{addr}/file?size=10000"), &out)
        .await
        .unwrap();
    wait_for(&manager, id, Status::Completed).await;
    manager.remove(id, true).await.unwrap();
    assert!(manager.list().await.unwrap().is_empty());
    assert!(names(&out).is_empty());
    assert!(manager.remove(id, false).await.is_err());
}

#[tokio::test]
async fn removing_a_running_download_cleans_up() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let out = dir.path().join("out");
    let id = manager
        .add(
            format!("http://{addr}/file?size=2000000&limit=1000000"),
            &out,
        )
        .await
        .unwrap();
    wait_for_bytes(&manager, id, 200_000).await;
    manager.remove(id, false).await.unwrap();
    let start = Instant::now();
    while !names(&out).is_empty() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            names(&out)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// An address nothing listens on.
async fn dead_address() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    listener.local_addr().unwrap()
}

#[tokio::test]
async fn progress_lives_in_the_database_not_in_a_sidecar_file() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let out = dir.path().join("out");
    let manager = manager_at(&db);
    let id = manager
        .add(
            format!("http://{addr}/file?size=2000000&limit=1000000"),
            &out,
        )
        .await
        .unwrap();
    wait_for_bytes(&manager, id, 300_000).await;
    manager.pause(id).await.unwrap();
    wait_for(&manager, id, Status::Paused).await;

    assert_eq!(names(&out), ["mohona-test.bin.part"]);
    let record = Store::open(&db)
        .unwrap()
        .load_progress(id)
        .unwrap()
        .unwrap();
    assert_eq!(record.total, 2_000_000);
    assert!(record.segments.iter().map(|(s, c, _)| c - s).sum::<u64>() > 0);

    manager.resume(id).await.unwrap();
    wait_for(&manager, id, Status::Completed).await;
    assert!(Store::open(&db)
        .unwrap()
        .load_progress(id)
        .unwrap()
        .is_none());
    assert_eq!(names(&out), ["mohona-test.bin"]);
}

#[tokio::test]
async fn a_download_that_loses_the_network_tries_again_by_itself() {
    let dead = dead_address().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let mut events = manager.subscribe();
    let id = manager
        .add(
            format!("http://{dead}/file?size=200000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    wait_for(&manager, id, Status::Retrying).await;

    // The server comes back.
    let listener = bind(dead).await.unwrap();
    tokio::spawn(async move {
        let _ = serve(
            listener,
            Config {
                default_size: 1_000_000,
            },
        )
        .await;
    });
    wait_for(&manager, id, Status::Completed).await;
    let mut retrying = 0;
    while let Ok(e) = events.try_recv() {
        if matches!(
            e,
            ManagerEvent::State {
                status: Status::Retrying,
                ..
            }
        ) {
            retrying += 1;
        }
    }
    assert!(retrying >= 1);
}

#[tokio::test]
async fn it_gives_up_after_too_many_retries() {
    let dead = dead_address().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let mut events = manager.subscribe();
    let id = manager
        .add(
            format!("http://{dead}/file?size=200000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    wait_for(&manager, id, Status::Failed).await;
    let mut retrying = 0;
    while let Ok(e) = events.try_recv() {
        if matches!(
            e,
            ManagerEvent::State {
                status: Status::Retrying,
                ..
            }
        ) {
            retrying += 1;
        }
    }
    assert_eq!(retrying, 3, "max_retries is 3");
    // The person can still start it again by hand.
    manager.resume(id).await.unwrap();
    wait_for(&manager, id, Status::Failed).await;
}

#[tokio::test]
async fn a_download_waiting_to_retry_can_be_paused() {
    let dead = dead_address().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let id = manager
        .add(
            format!("http://{dead}/file?size=200000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    wait_for(&manager, id, Status::Retrying).await;
    manager.pause(id).await.unwrap();
    wait_for(&manager, id, Status::Paused).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rows = manager.list().await.unwrap();
    assert_eq!(
        rows[0].status,
        Status::Paused,
        "the timer must not restart it"
    );
}

async fn seconds_until_all_complete(manager: &Manager) -> f64 {
    let start = Instant::now();
    loop {
        let rows = manager.list().await.unwrap();
        if rows.iter().all(|r| r.status == Status::Completed) {
            return start.elapsed().as_secs_f64();
        }
        assert!(start.elapsed() < Duration::from_secs(60), "{rows:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_global_limit_covers_all_downloads_together() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    manager
        .set_speed_limit(SpeedTarget::Global, Some(1_000_000))
        .await
        .unwrap();
    let out = dir.path().join("out");
    for i in 0..2 {
        manager
            .add(format!("http://{addr}/file?size=1500000&n={i}"), &out)
            .await
            .unwrap();
    }
    let seconds = seconds_until_all_complete(&manager).await;
    assert!(
        (2.7..=3.6).contains(&seconds),
        "3 MB under a 1 MB/s global limit took {seconds:.2} s"
    );
}

#[tokio::test]
async fn queue_and_download_limits_apply_and_the_slowest_wins() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let out = dir.path().join("out");
    manager
        .set_speed_limit(SpeedTarget::Queue(1), Some(10_000_000))
        .await
        .unwrap();
    let id = manager
        .add(format!("http://{addr}/file?size=1000000"), &out)
        .await
        .unwrap();
    manager
        .set_speed_limit(SpeedTarget::Download(id), Some(500_000))
        .await
        .unwrap();
    let seconds = seconds_until_all_complete(&manager).await;
    assert!(
        (1.7..=2.6).contains(&seconds),
        "1 MB at 500 KB/s took {seconds:.2} s"
    );
}

#[tokio::test]
async fn a_running_download_follows_a_changed_limit_and_limits_are_saved() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let manager = manager_at(&db);
    manager
        .set_speed_limit(SpeedTarget::Global, Some(100_000))
        .await
        .unwrap();
    let id = manager
        .add(
            format!("http://{addr}/file?size=3000000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    wait_for(&manager, id, Status::Downloading).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    manager
        .set_speed_limit(SpeedTarget::Global, None)
        .await
        .unwrap();
    // At 100 KB/s this would take 30 s.
    let seconds = seconds_until_all_complete(&manager).await;
    assert!(seconds < 10.0, "{seconds:.2} s");
    assert_eq!(
        Store::open(&db).unwrap().setting("speed_limit").unwrap(),
        None
    );

    manager
        .set_speed_limit(SpeedTarget::Global, Some(42))
        .await
        .unwrap();
    assert_eq!(
        Store::open(&db)
            .unwrap()
            .setting("speed_limit")
            .unwrap()
            .as_deref(),
        Some("42")
    );
    assert!(manager
        .set_speed_limit(SpeedTarget::Download(999), Some(1))
        .await
        .is_err());
}
