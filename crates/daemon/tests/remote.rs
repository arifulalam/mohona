//! The remote web companion: its page, its login, and what a phone may and may not do.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::{serve, Call, Client, DaemonInfo, Response, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use mohona_testserver::{bind, serve as serve_files, Config};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

async fn files() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve_files(
            listener,
            Config {
                default_size: 200_000,
            },
        )
        .await;
    });
    addr
}

struct Api {
    port: u16,
    remote: u16,
    token: String,
    info: std::path::PathBuf,
    downloads: std::path::PathBuf,
    shutdown: Arc<Notify>,
}

async fn start(dir: &Path) -> Api {
    let store = Store::open(&dir.join("test.db")).unwrap();
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
    let downloads = dir.join("dl");
    std::fs::create_dir_all(&downloads).unwrap();
    let shutdown = Arc::new(Notify::new());
    tokio::spawn(serve(
        manager,
        ServerConfig {
            info_path: info_path.clone(),
            default_dir: downloads.clone(),
            stream_program: None,
            shutdown: shutdown.clone(),
        },
    ));
    let started = Instant::now();
    loop {
        if let Ok(info) = DaemonInfo::read(&info_path) {
            if info.pid == std::process::id() && info.api_port != 0 {
                return Api {
                    port: info.api_port,
                    remote: std::env::var("MOHONA_REMOTE_PORT")
                        .unwrap()
                        .parse()
                        .unwrap(),
                    info: info_path.clone(),
                    downloads: downloads.clone(),
                    token: info.token,
                    shutdown,
                };
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "daemon did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// One raw HTTP request to `port`; returns the status, the whole reply text and the JSON body.
async fn raw(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> (u16, String, Value) {
    let mut text = format!("{method} {path} HTTP/1.1\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        text.push_str(&format!("Host: 192.168.1.20:{port}\r\n"));
    }
    for (k, v) in headers {
        text.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        text.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    text.push_str("\r\n");
    text.push_str(body.unwrap_or(""));
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(text.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out)).await;
    let reply = String::from_utf8_lossy(&out).to_string();
    let status = reply
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = reply
        .split_once("\r\n\r\n")
        .map_or("", |(_, b)| b)
        .to_string();
    let json = serde_json::from_str(&body).unwrap_or(Value::Null);
    (status, reply, json)
}

async fn remote(
    api: &Api,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let auth = format!("Bearer {token}");
    let (s, _, j) = raw(
        api.remote,
        method,
        path,
        &[("Authorization", &auth)],
        body.map(|b| b.to_string()).as_deref(),
    )
    .await;
    (s, j)
}

async fn make_token(api: &Api, scope: &str) -> String {
    let client = Client::new(&DaemonInfo::read(&api.info).unwrap());
    match client
        .call(Call::MakeToken {
            name: "phone".into(),
            scope: scope.into(),
        })
        .await
        .unwrap()
    {
        Response::Token { token, .. } => token,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_companion_serves_its_page_and_limits_what_a_phone_can_do() {
    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    std::env::set_var("MOHONA_REMOTE_PORT", free.to_string());
    let dir = tempfile::tempdir().unwrap();
    let api = start(dir.path()).await;
    let file_server = files().await;

    // The page needs no login, and may not load anything from elsewhere.
    let (s, page, _) = raw(api.remote, "GET", "/", &[], None).await;
    assert_eq!(s, 200);
    assert!(
        page.contains("Mohona remote") && page.contains("default-src 'none'"),
        "{page}"
    );

    // Everything else needs a token; the master token is for this computer only.
    assert_eq!(remote(&api, "", "GET", "/v1/downloads", None).await.0, 401);
    let (s, b) = remote(&api, &api.token, "GET", "/v1/downloads", None).await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (403, Some("master_token_local_only"))
    );
    // ... and the local API still takes it.
    let (s, _, _) = raw(
        api.port,
        "GET",
        "/v1/downloads",
        &[
            ("Host", &format!("127.0.0.1:{}", api.port)),
            ("Authorization", &format!("Bearer {}", api.token)),
        ],
        None,
    )
    .await;
    assert_eq!(s, 200);

    // A read token looks; it does not add.
    let look = make_token(&api, "read").await;
    assert_eq!(
        remote(&api, &look, "GET", "/v1/downloads", None).await.0,
        200
    );
    assert_eq!(
        remote(
            &api,
            &look,
            "POST",
            "/v1/downloads",
            Some(json!({ "url": "http://x.invalid/a" }))
        )
        .await
        .0,
        403
    );

    // A write token adds and controls downloads, but cannot choose the folder nor administer.
    let write = make_token(&api, "write").await;
    let other = dir.path().join("elsewhere");
    let url = format!("http://{file_server}/file");
    let (s, b) = remote(
        &api,
        &write,
        "POST",
        "/v1/downloads",
        Some(json!({ "url": url, "folder": other.display().to_string() })),
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let id = b["id"].as_i64().unwrap();
    let started = Instant::now();
    let path = loop {
        let (_, list) = remote(&api, &write, "GET", "/v1/downloads", None).await;
        let found = list["downloads"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .cloned();
        if let Some(d) = found {
            if d["status"] == "completed" {
                break d["path"].as_str().unwrap().to_string();
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "did not finish"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(
        std::path::Path::new(&path).starts_with(&api.downloads),
        "{path}"
    );
    assert!(!other.exists());
    // Removing never deletes the file from a phone.
    assert_eq!(
        remote(
            &api,
            &write,
            "DELETE",
            &format!("/v1/downloads/{id}?delete_file=true"),
            None
        )
        .await
        .0,
        200
    );
    assert!(std::path::Path::new(&path).exists());
    assert_eq!(
        remote(&api, &write, "PUT", "/v1/config", Some(json!({})))
            .await
            .0,
        403
    );
    assert_eq!(remote(&api, &write, "GET", "/v1/tokens", None).await.0, 403);
    assert_eq!(remote(&api, &write, "GET", "/v1/events", None).await.0, 404);

    // Another web page cannot use a phone's token; the companion's own page can.
    let auth = format!("Bearer {write}");
    let host = format!("192.168.1.20:{}", api.remote);
    let (s, _, _) = raw(
        api.remote,
        "GET",
        "/v1/downloads",
        &[("Authorization", &auth), ("Origin", "http://evil.example")],
        None,
    )
    .await;
    assert_eq!(s, 403);
    let origin = format!("http://{host}");
    let (s, _, _) = raw(
        api.remote,
        "GET",
        "/v1/downloads",
        &[
            ("Authorization", &auth),
            ("Origin", &origin),
            ("Sec-Fetch-Site", "same-origin"),
        ],
        None,
    )
    .await;
    assert_eq!(s, 200);
    api.shutdown.notify_one();
}
