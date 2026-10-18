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

/// A page that needs cookies, a referrer and a header: `get` and `queue add`
/// work with the flags and fail without them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookies_referer_and_headers_from_the_command_line() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let url = format!("http://{addr}/guard?size=200000");
    let flags = [
        "--cookie",
        "session=ok",
        "--referer",
        "https://example.org/page",
        "-H",
        "X-Token: abc",
    ];
    let run = |args: Vec<String>| {
        let db = db.clone();
        tokio::task::spawn_blocking(move || mohona(&db).args(args).output().unwrap())
    };
    let strings = |parts: &[&str]| parts.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    // `get` without the details is refused, and creates nothing.
    let out = dir.path().join("get-out");
    let mut args = strings(&["get", &url, "-o"]);
    args.push(out.to_string_lossy().into_owned());
    assert!(!run(args.clone()).await.unwrap().status.success());
    // With them it works.
    args.extend(strings(&flags));
    assert!(run(args).await.unwrap().status.success());
    assert_eq!(
        std::fs::read(out.join("mohona-test.bin")).unwrap().len(),
        200_000
    );

    // The same for the queue; the details are kept with the download.
    let out = dir.path().join("queue-out");
    let mut args = strings(&["queue", "add", &url, "-o"]);
    args.push(out.to_string_lossy().into_owned());
    args.extend(strings(&flags));
    assert!(run(args).await.unwrap().status.success());
    assert!(run(strings(&["queue", "run"]))
        .await
        .unwrap()
        .status
        .success());
    let store = Store::open(&db).unwrap();
    let rows = store.list_downloads().unwrap();
    assert_eq!(rows[0].status, Status::Completed);
    // The cookie is not shown in `queue list`.
    let listing = run(strings(&["queue", "list"])).await.unwrap();
    assert!(!String::from_utf8_lossy(&listing.stdout).contains("session=ok"));
}
