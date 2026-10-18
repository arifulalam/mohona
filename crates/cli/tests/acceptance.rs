//! Acceptance tests that run the real program, like a person would.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mohona_store::{Status, Store};
use mohona_testserver::{bind, content_byte, serve, Config};

const MOHONA: &str = env!("CARGO_BIN_EXE_mohona");

async fn start_server() -> std::net::SocketAddr {
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

fn mohona(db: &Path) -> Command {
    let mut command = Command::new(MOHONA);
    command.arg("--db").arg(db);
    command
}

/// Kill the program while twenty downloads run at once, start it again, and
/// check that every file is complete and correct.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn killing_the_app_during_20_downloads_loses_nothing() {
    const COUNT: usize = 20;
    const SIZE: usize = 4_000_000;
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let out = dir.path().join("out");

    let mut add = mohona(&db);
    add.args(["queue", "add", "-o"]).arg(&out);
    for i in 0..COUNT {
        // About 400 KB/s per connection: slow enough to be caught midway.
        add.arg(format!("http://{addr}/file?size={SIZE}&limit=400000&n={i}"));
    }
    assert!(add.output().unwrap().status.success());

    let mut child = mohona(&db)
        .args(["queue", "run", "-s", "20"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // Wait until every download has saved some progress, then pull the plug.
    let store = Store::open(&db).unwrap();
    let start = Instant::now();
    loop {
        let rows = store.list_downloads().unwrap();
        let running = rows
            .iter()
            .filter(|r| r.status.is_active() && r.downloaded > 0)
            .count();
        let done = rows
            .iter()
            .filter(|r| r.status == Status::Completed)
            .count();
        if running + done == COUNT && running > 0 {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "downloads did not all get going"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    child.kill().unwrap(); // no goodbye: like a crash or power cut
    child.wait().unwrap();

    let rows = store.list_downloads().unwrap();
    let unfinished = rows
        .iter()
        .filter(|r| r.status != Status::Completed)
        .count();
    assert!(
        unfinished > 0,
        "everything finished before the kill; test is too fast"
    );

    // Start again: the list carries on from where it was.
    let status = tokio::task::spawn_blocking({
        let db = db.clone();
        move || {
            mohona(&db)
                .args(["queue", "run", "-s", "20"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
        }
    })
    .await
    .unwrap();
    assert!(status.success());

    let rows = store.list_downloads().unwrap();
    assert_eq!(rows.len(), COUNT);
    assert!(rows.iter().all(|r| r.status == Status::Completed));
    let mut names: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names.len(),
        COUNT,
        "only finished files are left: {names:?}"
    );
    for row in rows {
        let bytes = std::fs::read(row.path.unwrap()).unwrap();
        assert_eq!(bytes.len(), SIZE);
        assert!(
            bytes
                .iter()
                .enumerate()
                .all(|(i, b)| *b == content_byte(i as u64)),
            "download #{} has wrong content",
            row.id
        );
    }
}
