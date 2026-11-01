//! The public HTTP API: its contract with the OpenAPI file, and its safety rules.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::api::ROUTES;
use mohona_daemon::{serve, DaemonInfo, ServerConfig};
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
    token: String,
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
            default_dir: downloads,
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

/// One raw HTTP request; returns the status and the JSON body.
async fn raw(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> (u16, Value) {
    let mut text = format!("{method} {path} HTTP/1.1\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        text.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (k, v) in headers {
        text.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        if !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            text.push_str("Content-Type: application/json\r\n");
        }
        if !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        {
            text.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
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
    let body = reply.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

async fn call(
    api: &Api,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let auth = format!("Bearer {token}");
    raw(
        api.port,
        method,
        path,
        &[("Authorization", &auth)],
        body.map(|b| b.to_string()).as_deref(),
    )
    .await
}

async fn admin(api: &Api, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    call(api, &api.token, method, path, body).await
}

async fn until_done(api: &Api, id: i64) {
    let started = Instant::now();
    loop {
        let (_, d) = admin(api, "GET", &format!("/v1/downloads/{id}"), None).await;
        if d["status"] == "completed" {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "download did not finish: {d}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn the_openapi_file_describes_exactly_the_routes_the_server_has() {
    let spec: Value = serde_json::from_str(include_str!("../openapi.json")).unwrap();
    let mut documented = Vec::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            documented.push((
                method.to_uppercase(),
                path.clone(),
                op["operationId"].as_str().unwrap().to_string(),
            ));
        }
    }
    let mut real: Vec<_> = ROUTES
        .iter()
        .map(|r| (r.method.to_string(), r.path.to_string(), r.name.to_string()))
        .collect();
    documented.sort();
    real.sort();
    assert_eq!(documented, real);
    // Scopes agree too.
    for r in ROUTES {
        let scope = spec["paths"][r.path][r.method.to_lowercase()]["x-scope"]
            .as_str()
            .unwrap();
        assert_eq!(scope, format!("{:?}", r.scope).to_lowercase(), "{}", r.name);
    }
}

#[tokio::test]
async fn every_operation_answers_as_the_contract_says() {
    let dir = tempfile::tempdir().unwrap();
    let api = start(dir.path()).await;
    let server = files().await;

    // No token needed.
    let (s, b) = raw(api.port, "GET", "/v1/ping", &[], None).await;
    assert_eq!((s, b["api"].clone()), (200, json!(1)));
    let (s, b) = raw(api.port, "GET", "/v1/openapi.json", &[], None).await;
    assert_eq!((s, b["openapi"].clone()), (200, json!("3.0.3")));

    // Downloads.
    let (s, b) = admin(
        &api,
        "POST",
        "/v1/downloads",
        Some(json!({ "url": format!("http://{server}/file?size=100000") })),
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let id = b["id"].as_i64().unwrap();
    until_done(&api, id).await;
    let (s, b) = admin(&api, "GET", "/v1/downloads", None).await;
    assert_eq!(s, 200);
    assert_eq!(b["downloads"][0]["id"], id);
    let (s, b) = admin(&api, "GET", &format!("/v1/downloads/{id}/details"), None).await;
    assert_eq!((s, b["id"].clone()), (200, json!(id)));
    let (s, _) = admin(&api, "GET", "/v1/downloads/999", None).await;
    assert_eq!(s, 404);
    let (s, _) = admin(&api, "GET", "/v1/downloads/abc", None).await;
    assert_eq!(s, 400);
    for action in ["pause", "resume", "cancel"] {
        let (s, _) = admin(&api, "POST", &format!("/v1/downloads/{id}/{action}"), None).await;
        assert!(s == 200 || s == 400, "{action} {s}"); // finished: the engine may refuse
    }
    let (s, b) = admin(
        &api,
        "POST",
        "/v1/downloads",
        Some(json!({ "url": format!("http://{server}/file?size=100000"), "duplicates": "skip" })),
    )
    .await;
    assert_eq!(s, 409, "{b}");
    assert_eq!(b["error"]["code"], "duplicate");
    let (s, b) = admin(&api, "POST", "/v1/downloads", Some(json!({ "nope": 1 }))).await;
    assert_eq!(s, 400, "{b}");
    let (s, _) = admin(&api, "DELETE", &format!("/v1/downloads/{id}"), None).await;
    assert_eq!(s, 200);
    let (s, _) = admin(&api, "DELETE", &format!("/v1/downloads/{id}"), None).await;
    assert_eq!(s, 404);

    // Settings.
    let (s, b) = admin(
        &api,
        "PUT",
        "/v1/config",
        Some(json!({ "max_active": 3, "notifications": false })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = admin(&api, "GET", "/v1/config", None).await;
    assert_eq!((s, b["max_active"].clone()), (200, json!(3)));
    let (s, b) = admin(&api, "PUT", "/v1/config", Some(json!({ "max_active": 99 }))).await;
    assert_eq!(s, 400, "{b}");

    // Queues.
    let (s, b) = admin(&api, "POST", "/v1/queues", Some(json!({ "name": "Night" }))).await;
    assert_eq!(s, 200, "{b}");
    let (_, b) = admin(&api, "GET", "/v1/queues", None).await;
    let night = b["queues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["name"] == "Night")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (s, b) = admin(
        &api,
        "PUT",
        &format!("/v1/queues/{night}"),
        Some(json!({ "name": "Late", "max_active": 2 })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let (_, b) = admin(&api, "GET", "/v1/queues", None).await;
    assert!(b["queues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|q| q["name"] == "Late" && q["max_active"] == 2));
    let (s, _) = admin(&api, "DELETE", &format!("/v1/queues/{night}"), None).await;
    assert_eq!(s, 200);

    // Categories and plan.
    let (s, b) = admin(
        &api,
        "POST",
        "/v1/categories",
        Some(json!({ "name": "Fonts", "extensions": ["ttf", "otf"] })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let (_, b) = admin(&api, "GET", "/v1/categories", None).await;
    let fonts = b["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Fonts")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (s, _) = admin(&api, "DELETE", &format!("/v1/categories/{fonts}"), None).await;
    assert_eq!(s, 200);
    let (s, b) = admin(&api, "PUT", "/v1/plan", Some(json!({ "windows": [] }))).await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = admin(&api, "GET", "/v1/plan", None).await;
    assert_eq!((s, b["open_now"].clone()), (200, json!(true)));
    let (s, b) = admin(
        &api,
        "PUT",
        "/v1/plan/after-action",
        Some(json!({ "action": "sleep", "countdown_seconds": 600 })),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let (s, _) = admin(&api, "DELETE", "/v1/plan/after-action", None).await;
    assert_eq!(s, 200);

    // Every error has the same shape.
    let (s, b) = admin(&api, "GET", "/v1/nowhere", None).await;
    assert_eq!((s, b["error"]["code"].clone()), (404, json!("not_found")));
    let (s, b) = admin(&api, "PATCH", "/v1/downloads", None).await;
    assert_eq!(
        (s, b["error"]["code"].clone()),
        (405, json!("method_not_allowed"))
    );
    api.shutdown.notify_one();
}

#[tokio::test]
async fn a_request_from_a_web_page_or_with_a_wrong_host_or_token_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let api = start(dir.path()).await;
    let auth = format!("Bearer {}", api.token);
    let p = api.port;

    // No token, a wrong token: 401.
    assert_eq!(raw(p, "GET", "/v1/downloads", &[], None).await.0, 401);
    assert_eq!(
        raw(
            p,
            "GET",
            "/v1/downloads",
            &[("Authorization", "Bearer nope")],
            None
        )
        .await
        .0,
        401
    );
    assert_eq!(
        raw(
            p,
            "GET",
            "/v1/downloads",
            &[("Authorization", &format!("Basic {}", api.token))],
            None
        )
        .await
        .0,
        401
    );

    // A web page: its request carries Origin or Sec-Fetch-Site. Refused even with the right token,
    // and the answer gives no CORS permission.
    for headers in [
        vec![
            ("Origin", "https://evil.example"),
            ("Authorization", auth.as_str()),
        ],
        vec![("Origin", "null"), ("Authorization", auth.as_str())],
        vec![
            ("Sec-Fetch-Site", "cross-site"),
            ("Authorization", auth.as_str()),
        ],
        vec![
            ("Sec-Fetch-Site", "same-site"),
            ("Authorization", auth.as_str()),
        ],
    ] {
        let (s, b) = raw(
            p,
            "POST",
            "/v1/downloads",
            &headers,
            Some(r#"{"url":"http://x.test/a"}"#),
        )
        .await;
        assert_eq!(
            (s, b["error"]["code"].clone()),
            (403, json!("web_pages_not_allowed")),
            "{headers:?}"
        );
    }
    // A preflight (OPTIONS) is not served either.
    assert_eq!(
        raw(
            p,
            "OPTIONS",
            "/v1/downloads",
            &[
                ("Origin", "https://evil.example"),
                ("Access-Control-Request-Method", "POST")
            ],
            None
        )
        .await
        .0,
        403
    );
    // A command from a page's simple form post (no token) is refused too.
    assert_eq!(
        raw(
            p,
            "POST",
            "/v1/downloads",
            &[
                ("Content-Type", "text/plain"),
                ("Origin", "https://evil.example")
            ],
            Some("{\"url\":\"http://x.test/a\"}")
        )
        .await
        .0,
        403
    );

    // DNS rebinding: a foreign Host.
    for host in [
        "evil.example",
        "evil.example:80",
        &format!("evil.example:{p}"),
        &format!("127.0.0.1.evil.example:{p}"),
        "127.0.0.1",
    ] {
        let (s, b) = raw(p, "GET", "/v1/ping", &[("Host", host)], None).await;
        assert_eq!(
            (s, b["error"]["code"].clone()),
            (421, json!("bad_host")),
            "{host}"
        );
    }
    for host in [format!("localhost:{p}"), format!("[::1]:{p}")] {
        assert_eq!(
            raw(p, "GET", "/v1/ping", &[("Host", &host)], None).await.0,
            200,
            "{host}"
        );
    }

    // Body rules.
    let json_headers = [("Authorization", auth.as_str())];
    assert_eq!(
        raw(
            p,
            "POST",
            "/v1/downloads",
            &[("Authorization", &auth), ("Content-Type", "text/plain")],
            Some("{}")
        )
        .await
        .0,
        415
    );
    assert_eq!(
        raw(p, "POST", "/v1/downloads", &json_headers, Some("[1]"))
            .await
            .0,
        400
    );
    assert_eq!(
        raw(p, "POST", "/v1/downloads", &json_headers, Some("{not json"))
            .await
            .0,
        400
    );
    assert_eq!(
        raw(
            p,
            "POST",
            "/v1/downloads",
            &[("Authorization", &auth), ("Transfer-Encoding", "chunked")],
            Some("0\r\n\r\n")
        )
        .await
        .0,
        400
    );
    let big = "x".repeat(1024 * 1024 + 10);
    assert_eq!(
        raw(
            p,
            "POST",
            "/v1/downloads",
            &[
                ("Authorization", &auth),
                ("Content-Length", &big.len().to_string())
            ],
            None
        )
        .await
        .0,
        413
    );
    // Garbage that is not HTTP.
    let mut stream = TcpStream::connect(("127.0.0.1", p)).await.unwrap();
    stream
        .write_all(b"{\"type\":\"ping\"}\r\n\r\n")
        .await
        .unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out)).await;
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 400"));
    // The daemon still works afterwards.
    assert_eq!(raw(p, "GET", "/v1/ping", &[], None).await.0, 200);
    api.shutdown.notify_one();
}

#[tokio::test]
async fn tokens_have_scopes_are_kept_hashed_and_can_be_revoked() {
    let dir = tempfile::tempdir().unwrap();
    let api = start(dir.path()).await;
    let (s, b) = admin(
        &api,
        "POST",
        "/v1/tokens",
        Some(json!({ "name": "reader", "scope": "read" })),
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let read = b["token"].as_str().unwrap().to_string();
    let read_id = b["id"].as_i64().unwrap();
    let (_, b) = admin(
        &api,
        "POST",
        "/v1/tokens",
        Some(json!({ "name": "writer", "scope": "write" })),
    )
    .await;
    let write = b["token"].as_str().unwrap().to_string();
    assert_ne!(read, write);
    assert!(
        admin(
            &api,
            "POST",
            "/v1/tokens",
            Some(json!({ "name": "x", "scope": "root" }))
        )
        .await
        .0 == 400
    );
    assert!(
        admin(
            &api,
            "POST",
            "/v1/tokens",
            Some(json!({ "name": "", "scope": "read" }))
        )
        .await
        .0 == 400
    );

    // Read: can look, cannot change.
    assert_eq!(call(&api, &read, "GET", "/v1/downloads", None).await.0, 200);
    let (s, b) = call(
        &api,
        &read,
        "POST",
        "/v1/downloads",
        Some(json!({ "url": "http://x.test/a" })),
    )
    .await;
    assert_eq!(
        (s, b["error"]["code"].clone()),
        (403, json!("forbidden_scope"))
    );
    // Write: can add and remove, cannot change settings or make tokens.
    assert_eq!(call(&api, &write, "GET", "/v1/config", None).await.0, 200);
    assert_eq!(
        call(
            &api,
            &write,
            "PUT",
            "/v1/config",
            Some(json!({ "max_active": 2 }))
        )
        .await
        .0,
        403
    );
    assert_eq!(call(&api, &write, "GET", "/v1/tokens", None).await.0, 403);
    assert_eq!(
        call(
            &api,
            &write,
            "POST",
            "/v1/tokens",
            Some(json!({ "name": "n" }))
        )
        .await
        .0,
        403
    );

    // The list never shows secrets; the store never holds them.
    let (_, b) = admin(&api, "GET", "/v1/tokens", None).await;
    let text = b.to_string();
    assert!(!text.contains(&read) && !text.contains("hash"), "{text}");
    assert_eq!(b["tokens"].as_array().unwrap().len(), 2);
    let db = std::fs::read(dir.path().join("test.db")).unwrap();
    assert!(
        !db.windows(read.len()).any(|w| w == read.as_bytes()),
        "the secret is in the database"
    );

    // Revoked: refused at once.
    assert_eq!(
        admin(&api, "DELETE", &format!("/v1/tokens/{read_id}"), None)
            .await
            .0,
        200
    );
    assert_eq!(call(&api, &read, "GET", "/v1/downloads", None).await.0, 401);
    assert_eq!(
        admin(&api, "DELETE", &format!("/v1/tokens/{read_id}"), None)
            .await
            .0,
        404
    );
    api.shutdown.notify_one();
}

#[tokio::test]
async fn the_event_socket_needs_a_token_and_sends_events() {
    let dir = tempfile::tempdir().unwrap();
    let api = start(dir.path()).await;
    let server = files().await;
    let p = api.port;
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let upgrade = |auth: &str| {
        format!(
            "GET /v1/events HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n{auth}\r\n"
        )
    };
    // No token: refused before any upgrade.
    let mut s = TcpStream::connect(("127.0.0.1", p)).await.unwrap();
    s.write_all(upgrade("").as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 401"));
    // A web page's socket (Origin) is refused even with the token.
    let mut s = TcpStream::connect(("127.0.0.1", p)).await.unwrap();
    s.write_all(
        upgrade(&format!(
            "Origin: https://evil.example\r\nAuthorization: Bearer {}\r\n",
            api.token
        ))
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 403"));

    // With the token: switches, says hello, then reports a new download.
    let mut s = TcpStream::connect(("127.0.0.1", p)).await.unwrap();
    s.write_all(upgrade(&format!("Authorization: Bearer {}\r\n", api.token)).as_bytes())
        .await
        .unwrap();
    let mut buf = vec![0u8; 4096];
    let n = s.read(&mut buf).await.unwrap();
    let head = String::from_utf8_lossy(&buf[..n]).to_string();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");

    let (st, b) = admin(
        &api,
        "POST",
        "/v1/downloads",
        Some(json!({ "url": format!("http://{server}/file?size=100000") })),
    )
    .await;
    assert_eq!(st, 201);
    let id = b["id"].as_i64().unwrap();
    let mut seen = String::new();
    let started = Instant::now();
    let mut got_hello = head.contains("hello");
    while started.elapsed() < Duration::from_secs(15)
        && !(seen.contains("\"completed\"") && got_hello)
    {
        let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0, "the socket closed");
        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
        got_hello |= chunk.contains("hello");
        seen.push_str(&chunk);
    }
    assert!(
        got_hello
            && seen.contains(&format!("\"id\":{id}"))
            && seen.contains("\"added\"")
            && seen.contains("\"completed\""),
        "{seen}"
    );
    // A client closing is answered and the daemon carries on.
    s.write_all(&[0x88, 0x80, 1, 2, 3, 4]).await.unwrap();
    assert_eq!(raw(p, "GET", "/v1/ping", &[], None).await.0, 200);
    api.shutdown.notify_one();
}

#[tokio::test]
async fn the_extensions_settings_calls_cannot_reach_the_token_store() {
    use mohona_daemon::{Call, Client, Response};
    let dir = tempfile::tempdir().unwrap();
    let api = start(dir.path()).await;
    let (s, _) = admin(
        &api,
        "POST",
        "/v1/tokens",
        Some(json!({ "name": "t", "scope": "read" })),
    )
    .await;
    assert_eq!(s, 201);
    let client = Client::from_info_file(&dir.path().join("daemon.json")).unwrap();
    for call in [
        Call::SettingGet {
            key: "api.tokens".into(),
        },
        Call::SettingSet {
            key: "api.tokens".into(),
            value: "[]".into(),
        },
    ] {
        assert!(matches!(
            client.call(call).await.unwrap(),
            Response::Error { .. }
        ));
    }
    api.shutdown.notify_one();
}
