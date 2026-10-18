//! The manager against the local test server.

use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use mohona_core::RetryPolicy;
use mohona_manager::{
    ActionRunner, AddRequest, AddResult, AfterAction, Clock, DownloadStatus as Status, Duplicate,
    DuplicatePolicy, Manager, ManagerConfig, ManagerEvent, Moment, Schedule, SpeedTarget, Window,
};
use mohona_manager::{Network, NetworkProbe, NetworkState};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use mohona_testserver::{bind, content_byte, serve, Config};
use std::sync::{Arc, Mutex};

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

/// A network that tests can switch.
#[derive(Clone)]
struct FakeNet(Arc<Mutex<NetworkState>>);

impl FakeNet {
    fn at(address: Option<&str>) -> Self {
        FakeNet(Arc::new(Mutex::new(NetworkState {
            v4: address.map(|a| a.parse().unwrap()),
            v6: None,
        })))
    }
    fn online() -> Self {
        Self::at(Some("192.168.1.10"))
    }
    fn set(&self, address: Option<&str>) {
        *self.0.lock().unwrap() = NetworkState {
            v4: address.map(|a| a.parse().unwrap()),
            v6: None,
        };
    }
}

impl NetworkProbe for FakeNet {
    fn state(&self) -> NetworkState {
        *self.0.lock().unwrap()
    }
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
        // The machine running the tests may have no network at all.
        network: Network(Arc::new(FakeNet::online())),
        network_check: Duration::from_millis(50),
        network_settle: Duration::from_millis(200),
        ..ManagerConfig::default()
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

/// A clock the test moves by hand.
#[derive(Clone)]
struct TestClock(std::sync::Arc<std::sync::Mutex<Moment>>);

impl TestClock {
    fn at(weekday: u8, hour: u16, minute: u16) -> Self {
        TestClock(std::sync::Arc::new(std::sync::Mutex::new(Moment::new(
            weekday, hour, minute,
        ))))
    }
    fn set(&self, weekday: u8, hour: u16, minute: u16) {
        *self.0.lock().unwrap() = Moment::new(weekday, hour, minute);
    }
    fn clock(&self) -> Clock {
        let inner = self.0.clone();
        Clock(std::sync::Arc::new(move || *inner.lock().unwrap()))
    }
}

fn scheduled_manager_at(db: &Path, clock: &TestClock) -> Manager {
    let config = ManagerConfig {
        clock: clock.clock(),
        schedule_check: Duration::from_millis(50),
        ..config()
    };
    Manager::start(Store::open(db).unwrap(), client(), config).unwrap()
}

fn night(limit: Option<u64>) -> Schedule {
    Schedule {
        windows: vec![Window {
            days: vec![],
            start: "01:00".into(),
            end: "07:00".into(),
            speed_limit: limit,
        }],
    }
}

#[tokio::test]
async fn a_queue_outside_its_window_does_not_start_and_starts_when_it_opens() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let clock = TestClock::at(0, 12, 0);
    let manager = scheduled_manager_at(&dir.path().join("m.db"), &clock);
    manager.set_schedule(1, Some(night(None))).await.unwrap();
    let id = manager
        .add(
            format!("http://{addr}/file?size=100000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(manager.list().await.unwrap()[0].status, Status::Queued);

    clock.set(0, 1, 30);
    wait_for(&manager, id, Status::Completed).await;
}

#[tokio::test]
async fn closing_the_window_stops_downloads_and_keeps_their_progress() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let clock = TestClock::at(0, 2, 0);
    let manager = scheduled_manager_at(&dir.path().join("m.db"), &clock);
    manager.set_schedule(1, Some(night(None))).await.unwrap();
    let mut events = manager.subscribe();
    let id = manager
        .add(
            format!("http://{addr}/file?size=2000000&limit=1000000"),
            &out,
        )
        .await
        .unwrap();
    wait_for_bytes(&manager, id, 300_000).await;

    clock.set(0, 7, 0);
    wait_for(&manager, id, Status::Queued).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let rows = manager.list().await.unwrap();
    assert_eq!(rows[0].status, Status::Queued, "stays stopped while closed");
    assert!(rows[0].downloaded > 0 && rows[0].downloaded < 2_000_000);
    assert!(names(&out).iter().any(|n| n.ends_with(".part")));

    clock.set(1, 1, 5);
    wait_for(&manager, id, Status::Completed).await;
    check_content(
        Path::new(manager.list().await.unwrap()[0].path.as_deref().unwrap()),
        2_000_000,
    );

    let mut changes = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let ManagerEvent::QueueWindow { open, .. } = e {
            changes.push(open);
        }
    }
    assert_eq!(changes, [false, true]);
}

#[tokio::test]
async fn the_window_can_carry_a_speed_limit() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let clock = TestClock::at(2, 3, 0);
    let manager = scheduled_manager_at(&dir.path().join("m.db"), &clock);
    manager
        .set_schedule(1, Some(night(Some(500_000))))
        .await
        .unwrap();
    manager
        .add(
            format!("http://{addr}/file?size=1000000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    let seconds = seconds_until_all_complete(&manager).await;
    assert!(
        (1.7..=2.6).contains(&seconds),
        "1 MB under the window's 500 KB/s took {seconds:.2} s"
    );
}

#[tokio::test]
async fn schedules_are_saved_checked_and_can_be_removed() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let clock = TestClock::at(0, 12, 0);
    let manager = scheduled_manager_at(&db, &clock);
    manager.set_schedule(1, Some(night(None))).await.unwrap();
    assert!(manager
        .set_schedule(1, Some(Schedule { windows: vec![] }))
        .await
        .is_err());
    assert!(manager.set_schedule(99, Some(night(None))).await.is_err());
    assert!(manager.queues().await.unwrap()[0].schedule_json.is_some());
    manager.shutdown().await;

    // After a restart the queue is still closed at noon.
    let manager = scheduled_manager_at(&db, &clock);
    let id = manager
        .add(
            format!("http://{addr}/file?size=50000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(manager.list().await.unwrap()[0].status, Status::Queued);

    manager.set_schedule(1, None).await.unwrap();
    wait_for(&manager, id, Status::Completed).await;
}

/// A runner that only writes down what it was asked to do.
#[derive(Default)]
struct FakeRunner {
    done: Mutex<Vec<AfterAction>>,
    fail: bool,
}

impl ActionRunner for FakeRunner {
    fn run(&self, action: &AfterAction) -> Result<(), String> {
        self.done.lock().unwrap().push(action.clone());
        if self.fail {
            Err("no permission".into())
        } else {
            Ok(())
        }
    }
}

fn action_manager(db: &Path, runner: Arc<FakeRunner>) -> Manager {
    let config = ManagerConfig {
        runner: mohona_manager::Runner(runner),
        countdown_tick: Duration::from_millis(100),
        ..config()
    };
    Manager::start(Store::open(db).unwrap(), client(), config).unwrap()
}

async fn next_event(
    events: &mut tokio::sync::broadcast::Receiver<ManagerEvent>,
    wanted: impl Fn(&ManagerEvent) -> bool,
) -> ManagerEvent {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let event = events.recv().await.unwrap();
            if wanted(&event) {
                return event;
            }
        }
    })
    .await
    .expect("the expected event never came")
}

#[tokio::test]
async fn the_action_runs_after_a_visible_countdown_when_everything_is_done() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let runner = Arc::new(FakeRunner::default());
    let manager = action_manager(&dir.path().join("m.db"), runner.clone());
    let mut events = manager.subscribe();
    manager
        .set_after_action(Some(AfterAction::Shutdown), 3)
        .await
        .unwrap();
    for i in 0..2 {
        manager
            .add(
                format!("http://{addr}/file?size=200000&n={i}"),
                dir.path().join("out"),
            )
            .await
            .unwrap();
    }
    let mut seconds = Vec::new();
    loop {
        match next_event(&mut events, |e| {
            matches!(
                e,
                ManagerEvent::ActionCountdown { .. } | ManagerEvent::ActionDone { .. }
            )
        })
        .await
        {
            ManagerEvent::ActionCountdown { seconds_left, .. } => seconds.push(seconds_left),
            ManagerEvent::ActionDone { action } => {
                assert_eq!(action, AfterAction::Shutdown);
                break;
            }
            _ => unreachable!(),
        }
    }
    assert_eq!(
        seconds,
        [3, 2, 1],
        "the countdown is visible, second by second"
    );
    assert_eq!(*runner.done.lock().unwrap(), [AfterAction::Shutdown]);
    // It happens once.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(runner.done.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancelling_the_countdown_stops_the_action() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let runner = Arc::new(FakeRunner::default());
    let manager = action_manager(&dir.path().join("m.db"), runner.clone());
    let mut events = manager.subscribe();
    manager
        .set_after_action(Some(AfterAction::Sleep), 5)
        .await
        .unwrap();
    manager
        .add(
            format!("http://{addr}/file?size=100000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    next_event(&mut events, |e| {
        matches!(e, ManagerEvent::ActionCountdown { .. })
    })
    .await;
    manager.cancel_action().await.unwrap();
    let event = next_event(&mut events, |e| {
        matches!(e, ManagerEvent::ActionCancelled { .. })
    })
    .await;
    assert_eq!(event, ManagerEvent::ActionCancelled { still_armed: false });
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(runner.done.lock().unwrap().is_empty());
}

#[tokio::test]
async fn new_work_during_the_countdown_stops_it_and_it_starts_again_later() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let runner = Arc::new(FakeRunner::default());
    let manager = action_manager(&dir.path().join("m.db"), runner.clone());
    let mut events = manager.subscribe();
    manager
        .set_after_action(Some(AfterAction::Exit), 10)
        .await
        .unwrap();
    manager
        .add(format!("http://{addr}/file?size=100000"), &out)
        .await
        .unwrap();
    next_event(&mut events, |e| {
        matches!(e, ManagerEvent::ActionCountdown { .. })
    })
    .await;

    // More to download: the countdown stops, the action stays armed.
    manager
        .add(
            format!("http://{addr}/file?size=2000000&limit=1000000&b=1"),
            &out,
        )
        .await
        .unwrap();
    let event = next_event(&mut events, |e| {
        matches!(e, ManagerEvent::ActionCancelled { .. })
    })
    .await;
    assert_eq!(event, ManagerEvent::ActionCancelled { still_armed: true });
    assert!(runner.done.lock().unwrap().is_empty());

    next_event(&mut events, |e| {
        matches!(e, ManagerEvent::ActionDone { .. })
    })
    .await;
    assert_eq!(*runner.done.lock().unwrap(), [AfterAction::Exit]);
    assert_eq!(manager.list().await.unwrap().len(), 2);
    assert!(manager
        .list()
        .await
        .unwrap()
        .iter()
        .all(|r| r.status == Status::Completed));
}

#[tokio::test]
async fn an_idle_manager_does_not_trigger_the_action_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let runner = Arc::new(FakeRunner::default());
    let manager = action_manager(&dir.path().join("m.db"), runner.clone());
    manager
        .set_after_action(Some(AfterAction::Shutdown), 0)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        runner.done.lock().unwrap().is_empty(),
        "nothing was downloaded"
    );
}

#[tokio::test]
async fn a_failing_action_is_reported() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let runner = Arc::new(FakeRunner {
        fail: true,
        ..FakeRunner::default()
    });
    let manager = action_manager(&dir.path().join("m.db"), runner);
    let mut events = manager.subscribe();
    manager
        .set_after_action(
            Some(AfterAction::Run {
                program: "x".into(),
                args: vec![],
            }),
            0,
        )
        .await
        .unwrap();
    manager
        .add(
            format!("http://{addr}/file?size=50000"),
            dir.path().join("out"),
        )
        .await
        .unwrap();
    let event = next_event(&mut events, |e| {
        matches!(e, ManagerEvent::ActionFailed { .. })
    })
    .await;
    assert_eq!(
        event,
        ManagerEvent::ActionFailed {
            message: "no permission".into()
        }
    );
}

#[tokio::test]
async fn it_sorts_finished_files_into_category_folders() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    manager
        .add_category("Data", vec!["bin".into()], Some("Data".into()))
        .await
        .unwrap();
    let mut request = AddRequest::new(format!("http://{addr}/file?size=5000"), dir.path());
    request.sort_into_categories = true;
    let AddResult::Added(id) = manager.add_checked(request).await.unwrap() else {
        panic!("not added");
    };
    wait_for(&manager, id, Status::Completed).await;
    let row = manager.list().await.unwrap().remove(0);
    let path = std::path::PathBuf::from(row.path.unwrap());
    assert_eq!(path, dir.path().join("Data").join("mohona-test.bin"));
    assert!(path.exists());
    assert!(!dir.path().join("mohona-test.bin").exists());
    assert!(row.category_id.is_some());
}

#[tokio::test]
async fn it_leaves_files_alone_without_sorting() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    manager
        .add_category("Data", vec!["bin".into()], None)
        .await
        .unwrap();
    let request = AddRequest::new(format!("http://{addr}/file?size=5000"), dir.path());
    let AddResult::Added(id) = manager.add_checked(request).await.unwrap() else {
        panic!("not added");
    };
    wait_for(&manager, id, Status::Completed).await;
    assert!(dir.path().join("mohona-test.bin").exists());
}

#[tokio::test]
async fn it_handles_duplicates_by_policy() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let url = format!("http://{addr}/file?size=5000");
    let AddResult::Added(first) = manager
        .add_checked(AddRequest::new(&url, dir.path()))
        .await
        .unwrap()
    else {
        panic!("not added");
    };
    wait_for(&manager, first, Status::Completed).await;

    // Ask: refused.
    let err = manager
        .add_checked(AddRequest::new(&url, dir.path()))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        mohona_manager::ManagerError::Duplicate(Duplicate::SameAddress { .. })
    ));

    // Skip: nothing added.
    let mut request = AddRequest::new(&url, dir.path());
    request.duplicates = DuplicatePolicy::Skip;
    assert!(matches!(
        manager.add_checked(request).await.unwrap(),
        AddResult::Skipped(_)
    ));
    assert_eq!(manager.list().await.unwrap().len(), 1);

    // Rename: second copy gets a number.
    let mut request = AddRequest::new(&url, dir.path());
    request.duplicates = DuplicatePolicy::Rename;
    let AddResult::Added(second) = manager.add_checked(request).await.unwrap() else {
        panic!("not added");
    };
    wait_for(&manager, second, Status::Completed).await;
    assert!(dir.path().join("mohona-test (1).bin").exists());

    // Overwrite: earlier downloads and their files are replaced.
    let mut request = AddRequest::new(&url, dir.path());
    request.duplicates = DuplicatePolicy::Overwrite;
    let AddResult::Added(third) = manager.add_checked(request).await.unwrap() else {
        panic!("not added");
    };
    wait_for(&manager, third, Status::Completed).await;
    assert!(dir.path().join("mohona-test.bin").exists());
}

#[tokio::test]
async fn it_notices_an_existing_file_with_the_same_name() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file"), b"mine").unwrap();
    let manager = manager_at(&dir.path().join("m.db"));
    let err = manager
        .add_checked(AddRequest::new(
            format!("http://{addr}/file?size=5000"),
            dir.path(),
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        mohona_manager::ManagerError::Duplicate(Duplicate::FileExists { .. })
    ));
}

#[tokio::test]
async fn it_keeps_a_log_and_can_be_searched() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let manager = manager_at(&db);
    let id = manager
        .add(format!("http://{addr}/file?size=5000"), dir.path())
        .await
        .unwrap();
    wait_for(&manager, id, Status::Completed).await;
    let missing = manager
        .add(format!("http://{addr}/missing"), dir.path())
        .await
        .unwrap();
    wait_for(&manager, missing, Status::Failed).await;

    let store = Store::open(&db).unwrap();
    let log = store.log_of(id).unwrap();
    assert_eq!(log.first().unwrap().message, "added");
    assert!(log.last().unwrap().message.starts_with("completed"));
    let failed = store.log_of(missing).unwrap();
    assert!(failed.iter().any(|l| l.level == "error"));

    let found = store
        .search(&mohona_store::HistoryQuery {
            text: Some("mohona-test".into()),
            status: Some(Status::Completed),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, id);
}

fn manager_with_network(db: &Path, net: &FakeNet) -> Manager {
    let config = ManagerConfig {
        network: Network(Arc::new(net.clone())),
        ..config()
    };
    Manager::start(Store::open(db).unwrap(), client(), config).unwrap()
}

#[tokio::test]
async fn it_pauses_when_the_network_changes_and_resumes_after() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let net = FakeNet::online();
    let manager = manager_with_network(&dir.path().join("m.db"), &net);
    let mut events = manager.subscribe();
    let id = manager
        .add(
            format!("http://{addr}/file?size=1000000&limit=250000"),
            dir.path(),
        )
        .await
        .unwrap();
    wait_for(&manager, id, Status::Downloading).await;
    wait_for_bytes(&manager, id, 50_000).await;

    // Wi-Fi switch: a different local address.
    net.set(Some("10.20.30.40"));
    wait_for(&manager, id, Status::Queued).await;
    let kept = manager.list().await.unwrap()[0].downloaded;
    assert!(kept > 0 && kept < 1_000_000, "progress is kept: {kept}");

    // After the settle time it starts again and finishes.
    wait_for(&manager, id, Status::Completed).await;
    let row = manager.list().await.unwrap().remove(0);
    let bytes = std::fs::read(row.path.unwrap()).unwrap();
    assert_eq!(bytes.len(), 1_000_000);
    assert!(bytes
        .iter()
        .enumerate()
        .all(|(i, b)| *b == content_byte(i as u64)));

    let mut changed = false;
    let mut resumed = false;
    while let Ok(event) = events.try_recv() {
        match event {
            ManagerEvent::NetworkChanged { online: true } => changed = true,
            ManagerEvent::NetworkResumed => resumed = changed,
            _ => {}
        }
    }
    assert!(changed && resumed);
}

#[tokio::test]
async fn it_waits_while_offline_and_goes_on_when_the_network_returns() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let net = FakeNet::at(None);
    let manager = manager_with_network(&dir.path().join("m.db"), &net);
    let id = manager
        .add(format!("http://{addr}/file?size=20000"), dir.path())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let row = manager.list().await.unwrap().remove(0);
    assert_eq!(
        row.status,
        Status::Queued,
        "nothing starts without a network"
    );

    net.set(Some("192.168.0.2"));
    wait_for(&manager, id, Status::Completed).await;
}

#[tokio::test]
async fn it_stops_downloads_when_the_network_goes_away() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let net = FakeNet::online();
    let manager = manager_with_network(&dir.path().join("m.db"), &net);
    let id = manager
        .add(
            format!("http://{addr}/file?size=1000000&limit=250000"),
            dir.path(),
        )
        .await
        .unwrap();
    wait_for_bytes(&manager, id, 50_000).await;
    net.set(None);
    wait_for(&manager, id, Status::Queued).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let row = manager.list().await.unwrap().remove(0);
    assert_eq!(row.status, Status::Queued, "stays stopped while offline");
    net.set(Some("192.168.1.10"));
    wait_for(&manager, id, Status::Completed).await;
}
