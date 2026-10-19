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

/// Logging in from the command line: `-u`, a login in the address, and the
/// password from the environment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logging_in_from_the_command_line() {
    let addr = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    let run = |args: Vec<String>, password: Option<&'static str>| {
        let db = db.clone();
        tokio::task::spawn_blocking(move || {
            let mut command = mohona(&db);
            command.args(args);
            command.env_remove("MOHONA_PASSWORD");
            if let Some(password) = password {
                command.env("MOHONA_PASSWORD", password);
            }
            command.output().unwrap()
        })
    };
    let get = |url: String, extra: &[&str], out: &str| {
        let mut args = vec!["get".to_string(), url, "-o".into(), out.to_string()];
        args.extend(extra.iter().map(|s| s.to_string()));
        args
    };
    let out = |name: &str| dir.path().join(name).to_string_lossy().into_owned();

    for (path, name) in [("auth", "basic"), ("digest", "digest")] {
        let url = format!("http://{addr}/{path}?size=120000");
        // No login: refused.
        let refused = run(get(url.clone(), &[], &out(&format!("{name}-none"))), None)
            .await
            .unwrap();
        assert!(!refused.status.success(), "{path} without login");
        // Wrong password: refused.
        let wrong = run(
            get(
                url.clone(),
                &["-u", "mohona:wrong"],
                &out(&format!("{name}-wrong")),
            ),
            None,
        )
        .await
        .unwrap();
        assert!(!wrong.status.success(), "{path} wrong password");
        // Right: -u, the address, or the environment.
        let ok = run(
            get(
                url.clone(),
                &["-u", "mohona:secret"],
                &out(&format!("{name}-u")),
            ),
            None,
        )
        .await
        .unwrap();
        assert!(ok.status.success(), "{path} -u");
        let inline = format!("http://mohona:secret@{addr}/{path}?size=120000");
        let ok = run(get(inline, &[], &out(&format!("{name}-inline"))), None)
            .await
            .unwrap();
        assert!(ok.status.success(), "{path} login in address");
        let ok = run(
            get(url, &["-u", "mohona"], &out(&format!("{name}-env"))),
            Some("secret"),
        )
        .await
        .unwrap();
        assert!(ok.status.success(), "{path} password from environment");
        for suffix in ["u", "inline", "env"] {
            let file = dir.path().join(format!("{name}-{suffix}/mohona-test.bin"));
            assert_eq!(std::fs::read(file).unwrap().len(), 120_000);
        }
    }

    // `-u name` with no MOHONA_PASSWORD is an error that says what to do.
    let url = format!("http://{addr}/auth?size=1000");
    let out_dir = out("noenv");
    let no_password = run(get(url, &["-u", "mohona"], &out_dir), None)
        .await
        .unwrap();
    assert!(!no_password.status.success());
    assert!(String::from_utf8_lossy(&no_password.stderr).contains("MOHONA_PASSWORD"));
}

#[test]
fn proxy_flags_send_the_download_through_the_proxy() {
    use mohona_testserver::proxy::start_http_proxy;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (server, proxy) = rt.block_on(async {
        let listener = mohona_testserver::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = mohona_testserver::serve(
                listener,
                mohona_testserver::Config {
                    default_size: 50_000,
                },
            )
            .await;
        });
        (addr, start_http_proxy(None).await)
    });
    let dir = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mohona"))
        .args(["get", &format!("http://{server}/file"), "-o"])
        .arg(dir.path())
        .args(["--proxy", &format!("http://{}", proxy.addr)])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(proxy.relayed() > 0);

    let before = proxy.relayed();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mohona"))
        .args(["get", &format!("http://{server}/file?size=1000"), "-o"])
        .arg(dir.path())
        .args(["--no-proxy"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(proxy.relayed(), before);
}
