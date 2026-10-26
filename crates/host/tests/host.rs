use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::{serve, ServerConfig};
use mohona_host::{read_message, write_message, Host, MAX_MESSAGE};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use serde_json::{json, Value};
use tokio::sync::Notify;

/// A real engine on its own thread, as `mohona daemon` would run it.
struct Engine {
    info: PathBuf,
    downloads: PathBuf,
    shutdown: Arc<Notify>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    fn start(dir: &Path) -> Self {
        Self::start_with(dir, None)
    }

    fn start_with(dir: &Path, stream_program: Option<PathBuf>) -> Self {
        let info = dir.join("daemon.json");
        let downloads = dir.join("dl");
        std::fs::create_dir_all(&downloads).unwrap();
        let shutdown = Arc::new(Notify::new());
        let (db, info2, dl2, stop) = (
            dir.join("mohona.db"),
            info.clone(),
            downloads.clone(),
            shutdown.clone(),
        );
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let client = HttpClient::new(&ClientConfig {
                    use_system_proxy: false,
                    ..ClientConfig::default()
                })
                .unwrap();
                let manager = Manager::start(
                    Store::open(&db).unwrap(),
                    client,
                    ManagerConfig {
                        adaptive: false,
                        ..ManagerConfig::default()
                    },
                )
                .unwrap();
                let _ = serve(
                    manager,
                    ServerConfig {
                        info_path: info2,
                        default_dir: dl2,
                        stream_program,
                        shutdown: stop,
                    },
                )
                .await;
            });
        });
        let started = Instant::now();
        while !info.exists() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "engine did not start"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        Self {
            info,
            downloads,
            shutdown,
            thread: Some(thread),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn host_for(info: &Path) -> Host {
    Host::new(info.to_path_buf(), None, None)
}

fn ask(host: &Host, message: Value) -> Value {
    host.handle(message.to_string().as_bytes())
}

/// A file server on its own thread; returns its address.
fn file_server() -> std::net::SocketAddr {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let listener = mohona_testserver::bind("127.0.0.1:0".parse().unwrap())
                    .await
                    .unwrap();
                tx.send(listener.local_addr().unwrap()).unwrap();
                let _ = mohona_testserver::serve(
                    listener,
                    mohona_testserver::Config {
                        default_size: 100_000,
                    },
                )
                .await;
            });
    });
    rx.recv().unwrap()
}

#[test]
fn hello_says_who_is_there() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let answer = ask(
        &host_for(&engine.info),
        json!({"type": "hello", "requestId": 7}),
    );
    assert_eq!(answer["type"], "hello");
    assert_eq!(answer["requestId"], 7);
    assert_eq!(answer["protocol"], 1);
    assert_eq!(answer["engineRunning"], true);
    assert_eq!(answer["engineVersion"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn without_an_engine_hello_still_answers_and_add_says_what_to_do() {
    let dir = tempfile::tempdir().unwrap();
    let host = host_for(&dir.path().join("nothing.json"));
    let hello = ask(&host, json!({"type": "hello"}));
    assert_eq!(hello["engineRunning"], false);
    assert_eq!(hello["engineVersion"], Value::Null);
    let add = ask(
        &host,
        json!({"type": "add", "url": "https://example.com/a.zip"}),
    );
    assert_eq!(add["code"], "engine_not_running", "{add}");
    // The host was given no mohona program to start.
    assert_eq!(
        ask(&host, json!({"type": "startEngine"}))["code"],
        "no_engine"
    );
}

#[test]
fn an_added_download_with_the_browsers_context_is_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let server = file_server();
    let host = host_for(&engine.info);
    // The /cookie path only serves requests that carry the cookie.
    let answer = ask(
        &host,
        json!({
            "type": "add", "requestId": "a1",
            "url": format!("http://{server}/cookie?size=30000"),
            "referrer": "https://example.com/downloads",
            "cookies": "session=ok",
            "userAgent": "Mozilla/5.0 Test",
            "headers": [{"name": "Accept-Language", "value": "bn"}],
        }),
    );
    assert_eq!(answer["type"], "added", "{answer}");
    assert_eq!(answer["requestId"], "a1");
    let started = Instant::now();
    loop {
        let files: Vec<_> = std::fs::read_dir(&engine.downloads)
            .unwrap()
            .flatten()
            .collect();
        if files.len() == 1 && files[0].metadata().unwrap().len() == 30_000 {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "not downloaded"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_same_address_twice_is_reported_not_added() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let host = host_for(&engine.info);
    let url = json!({"type": "add", "url": "https://example.com/a.iso"});
    assert_eq!(ask(&host, url.clone())["type"], "added");
    let second = ask(&host, url);
    assert_eq!(second["type"], "duplicate", "{second}");
    assert!(second["message"].as_str().unwrap().contains("already"));
}

#[test]
fn bad_requests_get_specific_errors_and_add_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let host = host_for(&engine.info);
    let code = |message: Value| ask(&host, message)["code"].as_str().unwrap().to_string();
    assert_eq!(code(json!({"type": "add"})), "bad_request");
    assert_eq!(
        code(json!({"type": "add", "url": "file:///etc/passwd"})),
        "bad_url"
    );
    assert_eq!(
        code(json!({"type": "add", "url": "javascript:alert(1)"})),
        "bad_url"
    );
    assert_eq!(
        code(json!({"type": "add", "url": "https://a.com/x", "cookies": "a=b\r\nX: y"})),
        "bad_request"
    );
    assert_eq!(
        code(json!({"type": "add", "url": "https://a.com/x", "cookies": 5})),
        "bad_request"
    );
    assert_eq!(
        code(json!({"type": "add", "url": "https://a.com/x", "headers": [{"name": "X"}]})),
        "bad_request"
    );
    assert_eq!(code(json!({"type": "nope"})), "unknown_type");
    assert_eq!(code(json!({"no": "type"})), "bad_request");
    assert_eq!(host.handle(b"not json")["code"], "bad_json");
    // Nothing reached the engine: a valid add is the first download.
    let id = ask(
        &host,
        json!({"type": "add", "url": "https://example.com/ok.bin"}),
    )["id"]
        .as_i64()
        .unwrap();
    assert_eq!(id, 1);
}

#[test]
fn a_file_the_browser_holds_is_saved_in_pieces() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let host = host_for(&engine.info);
    let b64 = |bytes: &[u8]| {
        // A tiny encoder, so the test needs no extra crate.
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    };
    let started = ask(
        &host,
        json!({"type": "saveStart", "name": "scan", "mime": "image/png", "requestId": 1}),
    );
    assert_eq!(started["type"], "saving", "{started}");
    let transfer = started["transfer"].as_u64().unwrap();
    let png: &[u8] = b"\x89PNG\r\n\x1a\nIHDR....";
    let (a, b) = png.split_at(5);
    for (i, part) in [a, b].into_iter().enumerate() {
        let r = ask(
            &host,
            json!({"type": "savePiece", "transfer": transfer, "index": i, "data": b64(part)}),
        );
        assert_eq!(r["type"], "piece", "{r}");
    }
    let done = ask(&host, json!({"type": "saveEnd", "transfer": transfer}));
    assert_eq!(done["type"], "saved", "{done}");
    assert_eq!(done["name"], "scan.png");
    assert_eq!(std::fs::read(done["path"].as_str().unwrap()).unwrap(), png);
    assert!(Path::new(done["path"].as_str().unwrap()).starts_with(&engine.downloads));
    let _ = std::io::stdout().flush();

    // Mistakes are reported and nothing is left behind.
    assert_eq!(
        ask(&host, json!({"type": "savePiece", "index": 0, "data": ""}))["code"],
        "bad_request"
    );
    assert_eq!(
        ask(&host, json!({"type": "saveStart", "name": "a\nb"}))["code"],
        "bad_request"
    );
    let t = ask(&host, json!({"type": "saveStart", "name": "x"}))["transfer"]
        .as_u64()
        .unwrap();
    assert_eq!(
        ask(
            &host,
            json!({"type": "savePiece", "transfer": t, "index": 3, "data": "AAAA"})
        )["code"],
        "engine"
    );
    assert_eq!(
        ask(&host, json!({"type": "saveEnd", "transfer": t}))["code"],
        "engine"
    );
    assert_eq!(std::fs::read_dir(&engine.downloads).unwrap().count(), 1);
}

#[test]
fn downloads_can_be_listed_and_refused_links_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let server = file_server();
    let host = host_for(&engine.info);
    let listed = ask(&host, json!({"type": "list", "requestId": 3}));
    assert_eq!(listed["type"], "downloads");
    assert_eq!(listed["requestId"], 3);
    assert_eq!(listed["downloads"], json!([]));

    let id = ask(
        &host,
        json!({"type": "add", "url": format!("http://{server}/file?size=1000")}),
    )["id"]
        .as_i64()
        .unwrap();
    let started = Instant::now();
    loop {
        let rows = ask(&host, json!({"type": "list"}));
        let row = &rows["downloads"][0];
        if row["status"] == "completed" {
            assert_eq!(row["expired"], false);
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{rows}");
        std::thread::sleep(Duration::from_millis(50));
    }
    // A finished download cannot take a new link, and the reason comes back.
    let refused = ask(
        &host,
        json!({"type": "refresh", "id": id, "url": "https://a/b"}),
    );
    assert_eq!(refused["code"], "refused", "{refused}");
    assert_eq!(
        ask(&host, json!({"type": "refresh", "url": "https://a/b"}))["code"],
        "bad_request"
    );
    assert_eq!(
        ask(
            &host,
            json!({"type": "refresh", "id": 1, "url": "ftp://a/b"})
        )["code"],
        "bad_url"
    );
}

#[test]
fn a_login_in_the_address_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let answer = ask(
        &host_for(&engine.info),
        json!({"type": "add", "url": "https://me:secret@example.com/p.zip"}),
    );
    assert_eq!(answer["type"], "added", "{answer}");
}

#[test]
fn framing_round_trips_and_refuses_huge_messages() {
    let mut buffer = Vec::new();
    write_message(&mut buffer, &json!({"hello": "বাংলা"})).unwrap();
    let mut input = &buffer[..];
    let body = read_message(&mut input).unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["hello"],
        "বাংলা"
    );
    assert!(read_message(&mut input).unwrap().is_none());

    let huge = ((MAX_MESSAGE + 1) as u32).to_ne_bytes();
    let error = read_message(&mut &huge[..]).unwrap_err();
    assert!(error.to_string().contains("limit"), "{error}");
    // A message cut short is an error, not a clean end.
    let mut cut = 10u32.to_ne_bytes().to_vec();
    cut.extend_from_slice(b"{\"a\"");
    assert!(read_message(&mut &cut[..]).is_err());
}

#[test]
fn the_program_talks_over_stdin_and_stdout() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let mut child = Command::new(env!("CARGO_BIN_EXE_mohona-host"))
        // The arguments a browser adds (Chrome: the extension's origin).
        .args(["chrome-extension://abcdefgh/", "--info"])
        .arg(&engine.info)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    write_message(&mut stdin, &json!({"type": "hello", "requestId": 1})).unwrap();
    let hello: Value =
        serde_json::from_slice(&read_message(&mut stdout).unwrap().unwrap()).unwrap();
    assert_eq!(hello["type"], "hello");
    write_message(
        &mut stdin,
        &json!({"type": "add", "url": "https://example.com/z.bin", "requestId": 2}),
    )
    .unwrap();
    let added: Value =
        serde_json::from_slice(&read_message(&mut stdout).unwrap().unwrap()).unwrap();
    assert_eq!(added["type"], "added");
    assert_eq!(added["requestId"], 2);
    // Closing the pipe ends the program, and it said nothing else on stdout.
    drop(stdin);
    let mut rest = Vec::new();
    stdout.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty());
    assert!(child.wait().unwrap().success());
    let _ = std::io::stderr().flush();
}

mod registration {
    use mohona_host::register::*;
    use std::path::{Path, PathBuf};

    fn places(dir: &Path, mac: bool) -> Places {
        Places {
            home: dir.join("home"),
            data: dir.join("data"),
            windows: false,
            mac,
        }
    }

    const ID: &str = "abcdefghijklmnopabcdefghijklmnop";

    #[test]
    fn chromium_manifests_name_the_origin_and_firefox_the_addon() {
        let host = Path::new("/opt/mohona/mohona-host");
        let chrome = manifest(Browser::Chrome, host, &[ID.to_string()]);
        assert_eq!(chrome["name"], "org.mohona.host");
        assert_eq!(chrome["type"], "stdio");
        assert_eq!(chrome["path"], "/opt/mohona/mohona-host");
        assert_eq!(
            chrome["allowed_origins"],
            serde_json::json!([format!("chrome-extension://{ID}/")])
        );
        assert!(chrome.get("allowed_extensions").is_none());
        let firefox = manifest(Browser::Firefox, host, &[FIREFOX_ID.to_string()]);
        assert_eq!(
            firefox["allowed_extensions"],
            serde_json::json!([FIREFOX_ID])
        );
        assert!(firefox.get("allowed_origins").is_none());
    }

    #[test]
    fn ids_are_checked() {
        assert!(valid_chromium_id(ID));
        assert!(!valid_chromium_id("short"));
        assert!(!valid_chromium_id(&ID.replace('a', "z")));
        assert!(valid_firefox_id("mohona@mohona.local"));
        assert!(!valid_firefox_id("bad id\"}"));
        assert!(!valid_firefox_id(""));
    }

    #[test]
    fn install_status_and_uninstall_use_each_browsers_folder() {
        let dir = tempfile::tempdir().unwrap();
        let host = PathBuf::from("/opt/mohona/mohona-host");
        for mac in [false, true] {
            let places = places(dir.path(), mac);
            for browser in BROWSERS {
                assert!(status(&places, browser).is_none());
                let allowed = if browser == Browser::Firefox {
                    FIREFOX_ID.to_string()
                } else {
                    ID.to_string()
                };
                let done =
                    install(&places, browser, &host, std::slice::from_ref(&allowed)).unwrap();
                assert!(done.manifest.starts_with(dir.path().join("home")));
                assert!(done.manifest.ends_with("org.mohona.host.json"));
                assert!(done.registry_key.is_none());
                let (program, ids) = status(&places, browser).unwrap();
                assert_eq!(program, host);
                assert_eq!(ids.len(), 1);
                assert!(ids[0].contains(&allowed));
            }
            assert!(places
                .manifest_path(Browser::Firefox)
                .to_string_lossy()
                .contains(if mac {
                    "Mozilla/NativeMessagingHosts"
                } else {
                    ".mozilla"
                }));
            for browser in BROWSERS {
                assert!(uninstall(&places, browser).unwrap());
                assert!(!uninstall(&places, browser).unwrap());
            }
        }
    }

    #[test]
    fn on_windows_the_manifest_lives_in_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut places = places(dir.path(), false);
        places.windows = true;
        let path = places.manifest_path(Browser::Edge);
        assert_eq!(path, dir.path().join("data/native-messaging/edge.json"));
    }
}

#[cfg(unix)]
mod streams {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A pretend `mohona` that records how it was called.
    fn fake_engine(dir: &Path) -> (PathBuf, PathBuf) {
        let record = dir.join("record.txt");
        let script = dir.join("mohona");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {r}\nprintf 'cookie=%s\\n' \"$MOHONA_COOKIE\" >> {r}\n",
                r = record.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, record)
    }

    fn wait_for(path: &Path) -> String {
        let started = Instant::now();
        loop {
            if let Ok(text) = std::fs::read_to_string(path) {
                if text.contains("cookie=") {
                    return text;
                }
            }
            assert!(started.elapsed() < Duration::from_secs(10), "never ran");
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    #[test]
    fn a_stream_is_started_with_the_cookie_in_the_environment_only() {
        let dir = tempfile::tempdir().unwrap();
        let (script, record) = fake_engine(dir.path());
        let engine = Engine::start_with(dir.path(), Some(script));
        let host = host_for(&engine.info);
        let answer = ask(
            &host,
            json!({
                "type": "stream", "requestId": 4,
                "url": "https://cdn.example.com/live/index.m3u8",
                "referrer": "https://example.com/",
                "cookies": "session=secret",
                "userAgent": "UA/1",
                "name": "Lesson 1",
                "headers": [{"name": "X-Token", "value": "t"}],
            }),
        );
        assert_eq!(answer["type"], "streamStarted", "{answer}");
        assert_eq!(answer["requestId"], 4);
        let text = wait_for(&record);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "stream");
        assert_eq!(lines.last().copied(), Some("cookie=session=secret"));
        assert_eq!(
            lines[lines.len() - 2],
            "https://cdn.example.com/live/index.m3u8"
        );
        assert!(text.contains("--referer=https://example.com/"));
        assert!(text.contains("--user-agent=UA/1"));
        assert!(text.contains("--name=Lesson 1"));
        assert!(text.contains("--header=X-Token: t"));
        // The secret is not among the arguments.
        assert!(!lines[..lines.len() - 1]
            .iter()
            .any(|l| l.contains("secret")));
    }

    #[test]
    fn bad_stream_requests_start_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (script, record) = fake_engine(dir.path());
        let engine = Engine::start_with(dir.path(), Some(script));
        let host = host_for(&engine.info);
        let code = |m: Value| ask(&host, m)["code"].as_str().unwrap().to_string();
        assert_eq!(
            code(json!({"type": "stream", "url": "ftp://a/x.m3u8"})),
            "bad_url"
        );
        assert_eq!(
            code(json!({"type": "stream", "url": "file:///x.m3u8"})),
            "bad_url"
        );
        assert_eq!(
            code(json!({"type": "stream", "url": "https://a/x.m3u8", "name": "../evil"})),
            "bad_request"
        );
        assert_eq!(
            code(json!({"type": "stream", "url": "https://a/x.m3u8", "name": "C:evil"})),
            "bad_request"
        );
        assert_eq!(code(json!({"type": "stream"})), "bad_request");
        let nobody = host_for(&dir.path().join("nothing.json"));
        assert_eq!(
            ask(
                &nobody,
                json!({"type": "stream", "url": "https://a/x.m3u8"})
            )["code"],
            "engine_not_running"
        );
        std::thread::sleep(Duration::from_millis(300));
        assert!(!record.exists());
    }
}

#[test]
fn the_extensions_settings_are_kept_by_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::start(dir.path());
    let host = host_for(&engine.info);
    let first = ask(&host, json!({"type": "getSettings", "requestId": 1}));
    assert_eq!(first["type"], "settings");
    assert_eq!(first["requestId"], 1);
    assert!(first["value"].is_null());
    let saved = ask(
        &host,
        json!({"type": "setSettings", "value": "{\"updated\":5}"}),
    );
    assert_eq!(saved["type"], "settingsSaved");
    assert_eq!(
        ask(&host, json!({"type": "getSettings"}))["value"],
        "{\"updated\":5}"
    );
    let bad = ask(&host, json!({"type": "setSettings", "value": 7}));
    assert_eq!(bad["code"], "bad_request");
}
