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

#[test]
fn certificate_flags_decide_whether_a_private_https_server_is_trusted() {
    use mohona_testserver::tls::{serve_tls, Certificates};
    let certificates = Certificates::new(false);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let port = rt.block_on(async {
        let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = Certificates::clone_for_server(&certificates);
        tokio::spawn(async move {
            let _ = serve_tls(
                listener,
                Config {
                    default_size: 30_000,
                },
                &served,
            )
            .await;
        });
        port
    });
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, &certificates.ca_pem).unwrap();
    let url = format!("https://localhost:{port}/file");
    let get = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_mohona"))
            .args(["get", &url, "-o"])
            .arg(dir.path().join("out"))
            .args(extra)
            .output()
            .unwrap()
    };

    let strict = get(&[]);
    assert!(!strict.status.success());
    let message = String::from_utf8_lossy(&strict.stderr);
    assert!(
        message.contains("certificate") && message.contains("--ca-file"),
        "{message}"
    );

    let with_ca = get(&["--ca-file", ca.to_str().unwrap()]);
    assert!(
        with_ca.status.success(),
        "{}",
        String::from_utf8_lossy(&with_ca.stderr)
    );

    let insecure = get(&["--insecure"]);
    assert!(
        insecure.status.success(),
        "{}",
        String::from_utf8_lossy(&insecure.stderr)
    );
}

#[test]
fn checksum_flags_check_the_file_and_keep_a_bad_one() {
    use sha2::{Digest, Sha256};
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(async {
        let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = serve(
                listener,
                Config {
                    default_size: 20_000,
                },
            )
            .await;
        });
        addr
    });
    let mut data = vec![0u8; 20_000];
    mohona_testserver::fill_content(0, &mut data);
    let real: String = Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let url = format!("http://{server}/file");
    let get = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_mohona"))
            .current_dir(dir.path())
            .args(["get", &url, "-o"])
            .arg(&out)
            .args(extra)
            .output()
            .unwrap()
    };

    let good = get(&["--checksum", &format!("sha256:{real}")]);
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    let said = String::from_utf8_lossy(&good.stdout);
    assert!(said.contains("Checked: sha256 matches"), "{said}");

    let bad = get(&["--checksum", &"cd".repeat(32)]);
    assert!(!bad.status.success());
    let message = String::from_utf8_lossy(&bad.stderr);
    assert!(
        message.contains("does not match its checksum") && message.contains("was kept at"),
        "{message}"
    );
    // Both files are there: the good one and the kept bad one.
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 2);

    let nonsense = get(&["--checksum", "nothex"]);
    assert!(!nonsense.status.success());
    assert!(String::from_utf8_lossy(&nonsense.stderr).contains("invalid checksum"));

    // One checksum cannot fit several files.
    let many = Command::new(env!("CARGO_BIN_EXE_mohona"))
        .current_dir(dir.path())
        .args([
            "queue",
            "add",
            &url,
            &format!("{url}?size=5000"),
            "--checksum",
            &real,
        ])
        .output()
        .unwrap();
    assert!(!many.status.success());
    assert!(String::from_utf8_lossy(&many.stderr).contains("one file"));

    // In the queue, a mismatch ends as failed (not retried) and the log says why.
    let add = Command::new(env!("CARGO_BIN_EXE_mohona"))
        .current_dir(dir.path())
        .args([
            "queue",
            "add",
            &format!("{url}?size=7000"),
            "-o",
            "q",
            "--checksum",
            &"ef".repeat(32),
        ])
        .output()
        .unwrap();
    assert!(add.status.success());
    let run = Command::new(env!("CARGO_BIN_EXE_mohona"))
        .current_dir(dir.path())
        .args(["queue", "run"])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(text.contains("does not match its checksum"), "{text}");
    assert!(!text.contains("retrying"), "{text}");
    let list = Command::new(env!("CARGO_BIN_EXE_mohona"))
        .current_dir(dir.path())
        .args(["queue", "list"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&list.stdout).contains("failed"));
    assert_eq!(std::fs::read_dir(dir.path().join("q")).unwrap().count(), 1);
}

#[test]
fn mirrors_are_used_by_get_and_saved_by_the_queue() {
    use mohona_testserver::proxy::start_http_proxy;
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (a, b, proxy) = rt.block_on(async {
        let mut servers = Vec::new();
        for _ in 0..2 {
            let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
            servers.push(listener.local_addr().unwrap());
            tokio::spawn(async move {
                let _ = serve(
                    listener,
                    Config {
                        default_size: 3_000_000,
                    },
                )
                .await;
            });
        }
        (servers[0], servers[1], start_http_proxy(None).await)
    });
    let dir = tempfile::tempdir().unwrap();
    let proxy_url = format!("http://{}", proxy.addr);
    let mirror = format!("http://{b}/file");
    let seen = |server: std::net::SocketAddr| {
        proxy
            .targets
            .lock()
            .unwrap()
            .iter()
            .filter(|t| **t == server.to_string())
            .count()
    };

    let got = Command::new(env!("CARGO_BIN_EXE_mohona"))
        .current_dir(dir.path())
        .args(["get", &format!("http://{a}/file"), "-o", "out"])
        .args(["--mirror", &mirror, "--proxy", &proxy_url, "-c", "4"])
        .output()
        .unwrap();
    assert!(
        got.status.success(),
        "{}",
        String::from_utf8_lossy(&got.stderr)
    );
    assert!(seen(a) >= 2 && seen(b) >= 2, "a {} b {}", seen(a), seen(b));

    let (before_a, before_b) = (seen(a), seen(b));
    let add = Command::new(env!("CARGO_BIN_EXE_mohona"))
        .current_dir(dir.path())
        .args([
            "queue",
            "add",
            &format!("http://{a}/file?size=3000000"),
            "-o",
            "q",
        ])
        .args([
            "--mirror",
            &format!("http://{b}/file?size=3000000"),
            "--proxy",
            &proxy_url,
        ])
        .output()
        .unwrap();
    assert!(add.status.success());
    let run = Command::new(env!("CARGO_BIN_EXE_mohona"))
        .current_dir(dir.path())
        .args(["queue", "run"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        seen(a) > before_a && seen(b) > before_b,
        "the saved mirror was used"
    );
}

#[test]
fn ftp_addresses_work_in_get_and_in_the_queue() {
    use mohona_testserver::ftp::{start_ftp, FtpConfig};
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(start_ftp(FtpConfig {
        size: 700_000,
        login: Some(("pat".into(), "secret".into())),
        ..FtpConfig::default()
    }));
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_mohona"))
            .current_dir(dir.path())
            .args(args)
            .env("MOHONA_PASSWORD", "secret")
            .output()
            .unwrap()
    };
    let url = format!("ftp://{}/pub/file.bin", server.addr);

    let refused = run(&["get", &url, "-o", "out"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("-u user:password"));

    let ok = run(&["get", &url, "-o", "out", "-u", "pat"]);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert_eq!(
        std::fs::metadata(dir.path().join("out/file.bin"))
            .unwrap()
            .len(),
        700_000
    );

    // In the address, and through the queue (the login is not kept in the list).
    let in_address = format!("ftp://pat:secret@{}/pub/two.bin", server.addr);
    assert!(run(&["queue", "add", &in_address, "-o", "q"])
        .status
        .success());
    let list = run(&["queue", "list"]);
    assert!(!String::from_utf8_lossy(&list.stdout).contains("secret"));
    let done = run(&["queue", "run"]);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    assert_eq!(
        std::fs::metadata(dir.path().join("q/two.bin"))
            .unwrap()
            .len(),
        700_000
    );
}

#[test]
fn ftps_uses_the_certificate_flags() {
    use mohona_testserver::ftp::{start_ftp, FtpConfig, FtpTls};
    use mohona_testserver::tls::Certificates;
    let certificates = Certificates::new(false);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(start_ftp(FtpConfig {
        size: 300_000,
        tls: Some(FtpTls {
            certificates: certificates.clone(),
            implicit: true,
        }),
        ..FtpConfig::default()
    }));
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, &certificates.ca_pem).unwrap();
    let url = format!("ftps://localhost:{}/x.bin", server.addr.port());
    let get = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_mohona"))
            .current_dir(dir.path())
            .args(["get", &url, "-o", "out"])
            .args(extra)
            .output()
            .unwrap()
    };
    let strict = get(&[]);
    assert!(!strict.status.success());
    assert!(String::from_utf8_lossy(&strict.stderr).contains("--ca-file"));
    let trusted = get(&["--ca-file", ca.to_str().unwrap()]);
    assert!(
        trusted.status.success(),
        "{}",
        String::from_utf8_lossy(&trusted.stderr)
    );
    assert!(get(&["--insecure"]).status.success());
    assert_eq!(
        std::fs::metadata(dir.path().join("out/x.bin"))
            .unwrap()
            .len(),
        300_000
    );
}

#[test]
fn sftp_flags_decide_trust_and_login() {
    use mohona_testserver::sftp::{generate_key_pair, start_sftp, SftpConfig};
    let (private, public) = generate_key_pair(Some("hunter2"));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(start_sftp(SftpConfig {
        size: 400_000,
        authorized_key: Some(public),
        ..SftpConfig::default()
    }));
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("key");
    std::fs::write(&key, private).unwrap();
    let known = dir.path().join("known_hosts");
    let port = server.addr.port();
    let url = format!("sftp://pat@127.0.0.1:{port}/data.bin");
    let run = |args: &[&str], envs: &[(&str, &str)]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mohona"));
        command
            .current_dir(dir.path())
            .args([
                "get",
                &url,
                "-o",
                "out",
                "--known-hosts",
                known.to_str().unwrap(),
            ])
            .args(args)
            .env_remove("MOHONA_PASSWORD")
            .env_remove("MOHONA_KEY_PASSPHRASE");
        for (k, v) in envs {
            command.env(k, v);
        }
        command.output().unwrap()
    };
    let text = |o: &std::process::Output| String::from_utf8_lossy(&o.stderr).into_owned();

    // Unknown server: told how to trust it, and refused.
    let unknown = run(&["-u", "pat:secret"], &[]);
    assert!(!unknown.status.success());
    assert!(
        text(&unknown).contains(&server.fingerprint),
        "{}",
        text(&unknown)
    );

    // Wrong fingerprint.
    let wrong = run(&["-u", "pat:secret", "--host-key", "SHA256:AAAA"], &[]);
    assert!(text(&wrong).contains("you gave with --host-key"));

    // Password login, remembering the server.
    let ok = run(&["-u", "pat:secret", "--trust-new-host"], &[]);
    assert!(ok.status.success(), "{}", text(&ok));
    assert!(std::fs::read_to_string(&known)
        .unwrap()
        .contains("ssh-ed25519"));

    // Key login: the passphrase comes from the environment only.
    let no_pass = run(&["--key", key.to_str().unwrap()], &[]);
    assert!(!no_pass.status.success());
    assert!(text(&no_pass).contains("MOHONA_KEY_PASSPHRASE"));
    let with_pass = run(
        &["--key", key.to_str().unwrap()],
        &[("MOHONA_KEY_PASSPHRASE", "hunter2")],
    );
    assert!(with_pass.status.success(), "{}", text(&with_pass));
    assert_eq!(server.logins.lock().unwrap().last().unwrap().1, "key");
    assert_eq!(
        std::fs::metadata(dir.path().join("out/data.bin"))
            .unwrap()
            .len(),
        400_000
    );
}

#[test]
fn webdav_addresses_work_with_a_login_in_get_and_in_the_queue() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(async {
        let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = serve(
                listener,
                Config {
                    default_size: 90_000,
                },
            )
            .await;
        });
        addr
    });
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_mohona"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let refused = run(&["get", &format!("webdav://{server}/auth"), "-o", "out"]);
    assert!(!refused.status.success());
    let ok = run(&[
        "get",
        &format!("webdav://{server}/auth"),
        "-o",
        "out",
        "-u",
        "mohona:secret",
    ]);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );

    let in_address = format!("dav://mohona:secret@{server}/auth?size=50000");
    assert!(run(&["queue", "add", &in_address, "-o", "q"])
        .status
        .success());
    let list = run(&["queue", "list"]);
    assert!(!String::from_utf8_lossy(&list.stdout).contains("secret"));
    assert!(run(&["queue", "run"]).status.success());
    assert_eq!(std::fs::read_dir(dir.path().join("q")).unwrap().count(), 1);
}

fn content(sizes: &[usize]) -> Vec<u8> {
    let mut all = Vec::new();
    for size in sizes {
        let mut piece = vec![0u8; *size];
        mohona_testserver::fill_content(0, &mut piece);
        all.extend(piece);
    }
    all
}

fn start_media_server() -> (tokio::runtime::Runtime, std::net::SocketAddr) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let addr = rt.block_on(start_server());
    (rt, addr)
}

fn stream(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(MOHONA)
        .current_dir(dir)
        .arg("stream")
        .args(args)
        .output()
        .unwrap()
}

fn text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn stream_lists_what_an_hls_master_offers() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let out = stream(
        dir.path(),
        &[&format!("http://{server}/hls/master.m3u8"), "--list"],
    );
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(shown.contains("3 qualities"), "{shown}");
    assert!(shown.contains("720p 2.5 Mbit/s"), "{shown}");
    assert!(shown.contains("(sound only)"), "{shown}");
    assert!(shown.contains("Audio: en (default), bn"), "{shown}");
    // Nothing is downloaded when only listing.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn stream_downloads_hls_video_and_its_audio() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("http://{server}/hls/master.m3u8");
    let out = stream(dir.path(), &[&url, "-o", "out", "--no-join"]);
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    // The name comes from the address ("hls"), the best quality and the default audio.
    assert_eq!(
        std::fs::read(dir.path().join("out/hls.ts")).unwrap(),
        content(&[1000, 1100, 1200])
    );
    assert_eq!(
        std::fs::read(dir.path().join("out/hls.audio.ts")).unwrap(),
        content(&[300, 310])
    );
    assert!(shown.contains("Quality: 720p"), "{shown}");
    assert!(shown.contains("two files"), "{shown}");
    // Only the two results are left (no .parts folders).
    assert_eq!(
        std::fs::read_dir(dir.path().join("out")).unwrap().count(),
        2
    );

    let out = stream(
        dir.path(),
        &[
            &url,
            "-o",
            "second",
            "-q",
            "360",
            "--audio",
            "bn",
            "--name",
            "clip",
            "--no-join",
        ],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read(dir.path().join("second/clip.ts")).unwrap(),
        content(&[500, 600, 700])
    );
    assert_eq!(
        std::fs::read(dir.path().join("second/clip.audio.ts")).unwrap(),
        content(&[400, 410])
    );

    let out = stream(
        dir.path(),
        &[&url, "-o", "third", "-q", "audio", "--name", "song"],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read(dir.path().join("third/song.ts")).unwrap(),
        content(&[100, 200])
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("third")).unwrap().count(),
        1
    );
}

#[test]
fn stream_downloads_dash_video_and_audio() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("http://{server}/dash/manifest.mpd");
    let listed = text(&stream(dir.path(), &[&url, "--list"]));
    assert!(listed.contains("Video: 720p 2.5 Mbit/s"), "{listed}");
    assert!(listed.contains("Audio: 0.1 Mbit/s [en]"), "{listed}");
    let out = stream(dir.path(), &[&url, "-o", "out", "-q", "720", "--no-join"]);
    assert!(out.status.success(), "{}", text(&out));
    // Names come from the address: "dash" (manifest is a generic name).
    assert_eq!(
        std::fs::read(dir.path().join("out/dash.mp4")).unwrap(),
        content(&[100, 200, 300])
    );
    assert_eq!(
        std::fs::read(dir.path().join("out/dash.audio.m4a")).unwrap(),
        content(&[15, 25, 35])
    );
}

#[test]
fn stream_without_ffmpeg_keeps_two_files_and_says_how_to_fix_it() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("http://{server}/hls/master.m3u8");
    let out = stream(
        dir.path(),
        &[&url, "-o", "out", "--ffmpeg", "no-such-folder"],
    );
    let shown = text(&out);
    // Not having ffmpeg is not a failure: the downloads are good.
    assert!(out.status.success(), "{shown}");
    assert!(shown.contains("ffmpeg was not found"), "{shown}");
    assert!(shown.contains("--ffmpeg"), "{shown}");
    assert!(dir.path().join("out/hls.ts").is_file());
    assert!(dir.path().join("out/hls.audio.ts").is_file());
}

#[test]
fn stream_joining_with_a_broken_ffmpeg_result_keeps_the_files() {
    // Whatever ffmpeg is installed here (or none), joining the test server's
    // fake media cannot work; the files must survive either way.
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("http://{server}/hls/master.m3u8");
    let out = stream(dir.path(), &[&url, "-o", "out"]);
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(
        shown.contains("ffmpeg could not join") || shown.contains("ffmpeg was not found"),
        "{shown}"
    );
    assert!(dir.path().join("out/hls.ts").is_file());
    assert!(dir.path().join("out/hls.audio.ts").is_file());
    assert!(!dir.path().join("out/hls.mp4").exists());
}

#[test]
fn stream_downloads_hls_subtitles_as_srt() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("http://{server}/hls/master.m3u8");
    let out = stream(
        dir.path(),
        &[&url, "-o", "out", "--no-join", "--subs", "en,bn"],
    );
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    let en = std::fs::read_to_string(dir.path().join("out/hls.en.srt")).unwrap();
    // Two glued segments; the cue that crosses the cut is there once.
    assert_eq!(
        en,
        "1\n00:00:01,000 --> 00:00:03,000\nHello <i>world</i>\n\n\
         2\n00:00:08,000 --> 00:00:12,000\nAcross the cut\n\n\
         3\n00:00:15,000 --> 00:00:16,500\nThe end & more\n\n"
    );
    let bn = std::fs::read_to_string(dir.path().join("out/hls.bn.srt")).unwrap();
    assert_eq!(bn, "1\n00:00:02,000 --> 00:00:04,000\nহ্যালো\n\n");
    // Video, audio and the two subtitle files; no leftovers.
    assert_eq!(
        std::fs::read_dir(dir.path().join("out")).unwrap().count(),
        4,
        "{shown}"
    );

    // A language the stream does not have says what it has.
    let out = stream(dir.path(), &[&url, "-o", "x", "--subs", "fr"]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("this stream has: en, bn"),
        "{}",
        text(&out)
    );
    // --embed-subs needs --subs.
    let out = stream(dir.path(), &[&url, "-o", "x", "--embed-subs"]);
    assert!(!out.status.success());
}

#[test]
fn stream_downloads_dash_subtitles_in_vtt_and_ttml() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("http://{server}/dash/manifest.mpd");
    let out = stream(
        dir.path(),
        &[&url, "-o", "out", "--no-join", "--subs", "all"],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out/dash.bn.srt")).unwrap(),
        "1\n00:00:01,000 --> 00:00:02,000\nদাশ\n\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out/dash.fr.srt")).unwrap(),
        "1\n00:00:03,000 --> 00:00:04,000\nBonjour\nle monde\n\n"
    );
}

#[test]
fn stream_records_a_live_stream_until_it_ends() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("http://{server}/live/index.m3u8");
    let out = stream(dir.path(), &[&url, "-o", "rec", "--name", "show"]);
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(shown.contains("Live stream: recording"), "{shown}");
    assert!(shown.contains("the stream ended"), "{shown}");
    // Six segments of 100, 110, ... bytes, each once and in order.
    assert_eq!(
        std::fs::read(dir.path().join("rec/show.ts")).unwrap(),
        content(&[100, 110, 120, 130, 140, 150])
    );

    // Recording again never replaces the first file.
    let out = stream(dir.path(), &[&url, "-o", "rec", "--name", "show"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(dir.path().join("rec/show (1).ts").is_file());
    assert_eq!(
        std::fs::read(dir.path().join("rec/show.ts")).unwrap().len(),
        content(&[100, 110, 120, 130, 140, 150]).len()
    );
}

#[test]
fn stream_stops_a_live_recording_at_the_time_limit() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    // A stream that never ends; 3 seconds of video are asked for.
    let url = format!("http://{server}/live/index.m3u8?noend=1");
    let out = stream(
        dir.path(),
        &[&url, "-o", "rec", "--name", "x", "--record-for", "3s"],
    );
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(shown.contains("time limit reached"), "{shown}");
    assert_eq!(
        std::fs::read(dir.path().join("rec/x.ts")).unwrap(),
        content(&[100, 110, 120])
    );
    let out = stream(dir.path(), &[&url, "--record-for", "soon"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("is not a time"), "{}", text(&out));
    let out = stream(dir.path(), &[&url, "--subs", "en", "--record-for", "1s"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("live streams"), "{}", text(&out));
}

#[test]
fn stream_explains_problems() {
    let (_rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let base = format!("http://{server}");
    let out = stream(dir.path(), &[&format!("{base}/hls/protected.m3u8")]);
    assert!(!out.status.success());
    assert!(text(&out).contains("DRM"), "{}", text(&out));

    let out = stream(
        dir.path(),
        &[&format!("{base}/hls/master.m3u8"), "--audio", "fr"],
    );
    assert!(!out.status.success());
    assert!(
        text(&out).contains("this stream has: en, bn"),
        "{}",
        text(&out)
    );

    let out = stream(
        dir.path(),
        &[&format!("{base}/hls/master.m3u8"), "-q", "hd"],
    );
    assert!(!out.status.success());
    assert!(text(&out).contains("not a quality"), "{}", text(&out));

    let out = stream(dir.path(), &[&format!("{base}/info")]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("neither an HLS playlist nor a DASH manifest"),
        "{}",
        text(&out)
    );

    let out = stream(dir.path(), &[&format!("{base}/hls/missing.m3u8")]);
    assert!(!out.status.success());
    assert!(text(&out).contains("404"), "{}", text(&out));
}

#[test]
fn preview_copies_the_complete_start_of_an_unfinished_download() {
    let dir = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let url = "http://example.com/movies/clip.mp4";
    let data: Vec<u8> = (0..100u8).collect();
    std::fs::write(dir.path().join("clip.mp4.part"), &data).unwrap();
    // Two pieces: the first has 40 of its 50 bytes, so only 40 are usable.
    let progress = mohona_core::Progress {
        version: 1,
        url: url.into(),
        final_url: url.into(),
        filename: "clip.mp4".into(),
        total: 100,
        etag: None,
        last_modified: None,
        segments: vec![(0, 40, 50), (50, 100, 100)],
    };
    progress
        .save(&dir.path().join("clip.mp4.part.mohona"))
        .unwrap();
    let out = Command::new(MOHONA)
        .env("TMPDIR", temp.path())
        .env("TEMP", temp.path())
        .env("TMP", temp.path())
        .args(["preview", url, "-o"])
        .arg(dir.path())
        .arg("--no-open")
        .output()
        .unwrap();
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(shown.contains("first 40 B"), "{shown}");
    assert_eq!(
        std::fs::read(temp.path().join("mohona-preview/clip.mp4")).unwrap(),
        data[..40]
    );
    // The download itself is untouched.
    assert_eq!(
        std::fs::read(dir.path().join("clip.mp4.part")).unwrap(),
        data
    );

    // An address nobody is downloading, and a file that is just a file.
    let out = Command::new(MOHONA)
        .args(["preview", "http://example.com/other.bin", "--no-open", "-o"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(text(&out).contains("nothing to preview"), "{}", text(&out));
    let plain = dir.path().join("plain.txt");
    std::fs::write(&plain, "x").unwrap();
    let out = Command::new(MOHONA)
        .arg("preview")
        .arg(&plain)
        .arg("--no-open")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
}

#[test]
fn convert_lists_presets_and_explains_problems() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(MOHONA)
        .args(["convert", "--list"])
        .output()
        .unwrap();
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    for name in ["mp3", "m4a", "mp4", "mp4-small"] {
        assert!(shown.contains(name), "{shown}");
    }
    let out = Command::new(MOHONA).arg("convert").output().unwrap();
    assert!(!out.status.success());
    let out = Command::new(MOHONA)
        .args(["convert", "nothing.mp4"])
        .output()
        .unwrap();
    assert!(text(&out).contains("is not a file"), "{}", text(&out));
    let file = dir.path().join("a.mp4");
    std::fs::write(&file, "x").unwrap();
    let out = Command::new(MOHONA)
        .arg("convert")
        .arg(&file)
        .args(["--preset", "gif"])
        .output()
        .unwrap();
    assert!(text(&out).contains("not a preset"), "{}", text(&out));
    let out = Command::new(MOHONA)
        .arg("convert")
        .arg(&file)
        .args(["--ffmpeg", "no-such-place"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        text(&out).contains("ffmpeg was not found"),
        "{}",
        text(&out)
    );
}

#[test]
fn convert_makes_an_mp3_and_never_replaces_a_file() {
    let Some(ffmpeg) = mohona_media::mux::find_ffmpeg(None) else {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("song.mkv");
    let made = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("sine=frequency=440:duration=1")
        .args(["-c:a", "aac"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(made.status.success());
    for expected in ["song.mp3", "song (1).mp3"] {
        let out = Command::new(MOHONA)
            .arg("convert")
            .arg(&source)
            .args(["--preset", "mp3"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", text(&out));
        assert!(dir.path().join(expected).is_file(), "{}", text(&out));
    }
}

/// The daemon runs `mohona stream` for the browser extension.
#[test]
fn the_daemon_downloads_a_stream_for_the_extension() {
    use mohona_daemon::{Call, Client, DaemonInfo, Response};
    let (rt, server) = start_media_server();
    let dir = tempfile::tempdir().unwrap();
    let info = dir.path().join("daemon.json");
    let out = dir.path().join("out");
    let mut daemon = mohona(&dir.path().join("m.db"))
        .arg("daemon")
        .arg("--info")
        .arg(&info)
        .arg("--folder")
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !DaemonInfo::read(&info).is_ok_and(|i| i.pid == daemon.id()) {
        assert!(started.elapsed() < Duration::from_secs(10), "no daemon");
        std::thread::sleep(Duration::from_millis(50));
    }
    let client = Client::from_info_file(&info).unwrap();
    let call = |call| rt.block_on(client.call(call)).unwrap();

    // Refused before anything runs.
    for url in [
        "file:///etc/passwd",
        "ftp://example.com/a.m3u8",
        "http://a/\r\nX: y",
    ] {
        let answer = call(Call::Stream {
            url: url.into(),
            dest_dir: None,
            cookies: None,
            referrer: None,
            user_agent: None,
            headers: vec![],
            name: None,
        });
        assert!(
            matches!(answer, Response::Error { .. }),
            "{url}: {answer:?}"
        );
    }
    let answer = call(Call::Stream {
        url: format!("http://{server}/hls/master.m3u8"),
        dest_dir: None,
        cookies: Some("session=ok".into()),
        referrer: Some("https://example.com/watch".into()),
        user_agent: Some("Test UA".into()),
        headers: vec![("Accept-Language".into(), "bn".into())],
        name: Some("Lesson 1".into()),
    });
    let Response::Started { job, .. } = answer else {
        panic!("{answer:?}");
    };
    let started = Instant::now();
    loop {
        let Response::Jobs { jobs } = call(Call::Jobs) else {
            panic!()
        };
        let row = jobs.iter().find(|j| j.id == job).unwrap();
        if row.status != "running" {
            assert_eq!(row.status, "finished", "{row:?}");
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "stuck");
        std::thread::sleep(Duration::from_millis(100));
    }
    let names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().any(|n| n.starts_with("Lesson 1")), "{names:?}");

    // A stream that does not exist fails, with the program's message.
    let Response::Started { job, .. } = call(Call::Stream {
        url: format!("http://{server}/hls/missing.m3u8"),
        dest_dir: None,
        cookies: None,
        referrer: None,
        user_agent: None,
        headers: vec![],
        name: None,
    }) else {
        panic!()
    };
    let started = Instant::now();
    loop {
        let Response::Jobs { jobs } = call(Call::Jobs) else {
            panic!()
        };
        let row = jobs.iter().find(|j| j.id == job).unwrap();
        if row.status != "running" {
            assert_eq!(row.status, "failed");
            assert!(row.message.is_some());
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "stuck");
        std::thread::sleep(Duration::from_millis(100));
    }
    call(Call::Shutdown);
    let _ = daemon.wait();
}
