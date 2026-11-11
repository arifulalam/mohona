//! The public local API, version 1: REST over HTTP on 127.0.0.1 and a
//! WebSocket for events. It is a thin skin over the same calls the app and
//! the command line use, so it can do nothing they cannot.
//!
//! Safety rules, all checked before anything is read from the body:
//! - it listens on 127.0.0.1 only, and only loopback peers are served;
//! - the `Host` header must name this address (blocks DNS rebinding);
//! - a request that carries `Origin`, or `Sec-Fetch-Site` other than `none`,
//!   comes from a web page and is refused, and no CORS header is ever sent;
//! - every call but `/v1/ping` and `/v1/openapi.json` needs a bearer token;
//!   tokens have a scope (`read`, `write`, `admin`) and are kept hashed;
//! - bodies are JSON only, at most 1 MiB; one request per connection.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Map, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::protocol::{Call, Response};
use crate::server::{handle, same_token, State};
use mohona_manager::ManagerEvent;

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 1024 * 1024;
const TOKENS_KEY: &str = "api.tokens";
const MAX_TOKENS: usize = 32;
/// The port the API tries first (any free one when it is taken).
pub const DEFAULT_PORT: u16 = 6791;
const OPENAPI: &str = include_str!("../openapi.json");

/// What a token may do. A larger scope includes the smaller ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    None,
    Read,
    Write,
    Admin,
}

impl Scope {
    fn parse(text: &str) -> Option<Scope> {
        match text {
            "read" => Some(Scope::Read),
            "write" => Some(Scope::Write),
            "admin" => Some(Scope::Admin),
            _ => None,
        }
    }
}

/// One operation of the API.
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    pub name: &'static str,
    pub scope: Scope,
}

macro_rules! routes {
    ($(($m:expr, $p:expr, $n:expr, $s:expr)),* $(,)?) => {
        &[$(Route { method: $m, path: $p, name: $n, scope: $s }),*]
    };
}

/// Every operation. The OpenAPI file must describe exactly these (a test checks).
pub const ROUTES: &[Route] = routes![
    ("GET", "/v1/ping", "ping", Scope::None),
    ("GET", "/v1/openapi.json", "openapi", Scope::None),
    ("GET", "/v1/events", "events", Scope::Read),
    ("GET", "/v1/downloads", "listDownloads", Scope::Read),
    ("POST", "/v1/downloads", "addDownload", Scope::Write),
    ("GET", "/v1/downloads/{id}", "getDownload", Scope::Read),
    (
        "DELETE",
        "/v1/downloads/{id}",
        "removeDownload",
        Scope::Write
    ),
    (
        "GET",
        "/v1/downloads/{id}/details",
        "downloadDetails",
        Scope::Read
    ),
    (
        "POST",
        "/v1/downloads/{id}/pause",
        "pauseDownload",
        Scope::Write
    ),
    (
        "POST",
        "/v1/downloads/{id}/resume",
        "resumeDownload",
        Scope::Write
    ),
    (
        "POST",
        "/v1/downloads/{id}/cancel",
        "cancelDownload",
        Scope::Write
    ),
    ("GET", "/v1/config", "getConfig", Scope::Read),
    ("PUT", "/v1/config", "setConfig", Scope::Admin),
    ("GET", "/v1/queues", "listQueues", Scope::Read),
    ("POST", "/v1/queues", "addQueue", Scope::Admin),
    ("PUT", "/v1/queues/{id}", "setQueue", Scope::Admin),
    ("DELETE", "/v1/queues/{id}", "removeQueue", Scope::Admin),
    ("GET", "/v1/categories", "listCategories", Scope::Read),
    ("POST", "/v1/categories", "addCategory", Scope::Admin),
    (
        "DELETE",
        "/v1/categories/{id}",
        "removeCategory",
        Scope::Admin
    ),
    ("POST", "/v1/resolve", "resolveLink", Scope::Read),
    ("POST", "/v1/torrent/files", "torrentFiles", Scope::Read),
    ("POST", "/v1/grab", "grabStart", Scope::Write),
    ("GET", "/v1/grab", "grabStatus", Scope::Read),
    ("DELETE", "/v1/grab", "grabStop", Scope::Write),
    ("GET", "/v1/rules", "getRules", Scope::Read),
    ("PUT", "/v1/rules", "setRules", Scope::Admin),
    ("POST", "/v1/rules/test", "testRules", Scope::Read),
    ("GET", "/v1/profiles", "getProfiles", Scope::Read),
    ("PUT", "/v1/profiles", "setProfiles", Scope::Admin),
    ("GET", "/v1/plan", "getPlan", Scope::Read),
    ("PUT", "/v1/plan", "setPlan", Scope::Admin),
    (
        "PUT",
        "/v1/plan/after-action",
        "setAfterAction",
        Scope::Admin
    ),
    (
        "DELETE",
        "/v1/plan/after-action",
        "cancelAfterAction",
        Scope::Admin
    ),
    ("GET", "/v1/tokens", "listTokens", Scope::Admin),
    ("POST", "/v1/tokens", "createToken", Scope::Admin),
    ("DELETE", "/v1/tokens/{id}", "deleteToken", Scope::Admin),
];

struct Req {
    /// Arrived on the remote companion's port (LAN), not the local API.
    remote: bool,
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct Reply {
    status: u16,
    body: Value,
}

fn reply(status: u16, body: Value) -> Reply {
    Reply { status, body }
}

fn failure(status: u16, code: &str, message: impl Into<String>) -> Reply {
    reply(
        status,
        json!({ "error": { "code": code, "message": message.into() } }),
    )
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        421 => "Misdirected Request",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    }
}

/// Listens until the daemon stops. Returns the port it listens on.
pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
    match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => Ok(listener),
        // A port that is taken must not stop the daemon: take any free one.
        Err(_) if port != 0 => TcpListener::bind(("127.0.0.1", 0)).await,
        Err(e) => Err(e),
    }
}

pub(crate) async fn serve(listener: TcpListener, state: Arc<State>) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            continue;
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(3600 * 24),
                connection(stream, state, port),
            )
            .await;
        });
    }
}

const REMOTE_PAGE: &str = include_str!("remote.html");

/// The remote companion: the same operations for write-scope tokens, plus its one web page, on the LAN.
pub async fn bind_remote(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("0.0.0.0", port)).await
}

pub(crate) async fn serve_remote(listener: TcpListener, state: Arc<State>) {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            continue;
        };
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(60),
                remote_connection(stream, state, port),
            )
            .await;
        });
    }
}

async fn remote_connection(
    mut stream: TcpStream,
    state: Arc<State>,
    port: u16,
) -> std::io::Result<()> {
    let mut request =
        match tokio::time::timeout(Duration::from_secs(10), read_request(&mut stream)).await {
            Ok(Ok(request)) => request,
            Ok(Err(reply)) => return write_reply(&mut stream, reply).await,
            Err(_) => return Ok(()),
        };
    request.remote = true;
    if request.method == "GET" && (request.path == "/" || request.path == "/index.html") {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n",
            REMOTE_PAGE.len()
        );
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(REMOTE_PAGE.as_bytes()).await?;
        return stream.shutdown().await;
    }
    // No event stream here (the page asks again every few seconds), and no local-only addresses.
    let answer = if request.path == "/v1/events" {
        failure(404, "not_found", "there is no such address")
    } else {
        match prepare(&state, &request, port).await {
            Ok((route, scope, params)) => run(&state, route, scope, params, request).await,
            Err(reply) => reply,
        }
    };
    write_reply(&mut stream, answer).await
}

async fn connection(mut stream: TcpStream, state: Arc<State>, port: u16) -> std::io::Result<()> {
    let request =
        match tokio::time::timeout(Duration::from_secs(10), read_request(&mut stream)).await {
            Ok(Ok(request)) => request,
            Ok(Err(reply)) => return write_reply(&mut stream, reply).await,
            Err(_) => return Ok(()),
        };
    // The event stream keeps the connection; everything else is one answer.
    if request.method == "GET" && request.path == "/v1/events" {
        return match prepare(&state, &request, port).await {
            Ok(_) => events(stream, state, request).await,
            Err(reply) => write_reply(&mut stream, reply).await,
        };
    }
    let answer = match prepare(&state, &request, port).await {
        Ok((route, scope, params)) => run(&state, route, scope, params, request).await,
        Err(reply) => reply,
    };
    write_reply(&mut stream, answer).await
}

async fn write_reply(stream: &mut TcpStream, reply: Reply) -> std::io::Result<()> {
    let body = serde_json::to_vec(&reply.body).unwrap_or_default();
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        reply.status,
        reason(reply.status),
        body.len()
    );
    if reply.status == 401 {
        head.push_str("WWW-Authenticate: Bearer\r\n");
    }
    if reply.status == 405 {
        head.push_str("Allow: GET, POST, PUT, DELETE\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await
}

async fn read_request(stream: &mut TcpStream) -> Result<Req, Reply> {
    let mut data = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if data.len() > MAX_HEAD {
            return Err(failure(
                431,
                "head_too_large",
                "the request's head is too large",
            ));
        }
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| failure(400, "bad_request", "could not read the request"))?;
        if n == 0 {
            return Err(failure(400, "bad_request", "the request ended too early"));
        }
        data.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&data[..head_end])
        .map_err(|_| failure(400, "bad_request", "the request is not text"))?;
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(failure(400, "bad_request", "not an HTTP request"));
    };
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return Err(failure(400, "bad_request", "not an HTTP/1 request"));
    }
    let mut headers = HashMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(failure(400, "bad_request", "a header has no colon"));
        };
        if headers
            .insert(name.trim().to_ascii_lowercase(), value.trim().to_string())
            .is_some()
        {
            return Err(failure(400, "bad_request", "a header is repeated"));
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(failure(
            400,
            "bad_request",
            "chunked bodies are not accepted; send a Content-Length",
        ));
    }
    let length: usize = match headers.get("content-length") {
        None => 0,
        Some(v) => v
            .parse()
            .map_err(|_| failure(400, "bad_request", "Content-Length is not a number"))?,
    };
    if length > MAX_BODY {
        return Err(failure(
            413,
            "body_too_large",
            "the body is larger than 1 MiB",
        ));
    }
    let mut body = data[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| failure(400, "bad_request", "could not read the body"))?;
        if n == 0 {
            return Err(failure(400, "bad_request", "the body ended too early"));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    let (path, query_text) = target.split_once('?').unwrap_or((target, ""));
    let query = query_text
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.to_string(), v.to_string())
        })
        .collect();
    Ok(Req {
        remote: false,
        method: method.to_string(),
        path: path.to_string(),
        query,
        headers,
        body,
    })
}

/// Whether `Host` names this server: 127.0.0.1, localhost or [::1], with this port.
pub fn host_ok(host: &str, port: u16) -> bool {
    let Some((name, p)) = host.rsplit_once(':') else {
        return false;
    };
    p == port.to_string()
        && matches!(
            name.to_ascii_lowercase().as_str(),
            "127.0.0.1" | "localhost" | "[::1]"
        )
}

fn match_route(method: &str, path: &str) -> Result<Option<(&'static Route, Params)>, ()> {
    let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let mut path_known = false;
    for route in ROUTES {
        let pattern: Vec<&str> = route.path.split('/').collect();
        if pattern.len() != segments.len() {
            continue;
        }
        let mut params = HashMap::new();
        let mut hit = true;
        for (want, got) in pattern.iter().zip(&segments) {
            if let Some(name) = want.strip_prefix('{').and_then(|w| w.strip_suffix('}')) {
                if got.is_empty() {
                    hit = false;
                    break;
                }
                params.insert(name, (*got).to_string());
            } else if want != got {
                hit = false;
                break;
            }
        }
        if !hit {
            continue;
        }
        path_known = true;
        if route.method == method {
            return Ok(Some((route, params)));
        }
    }
    if path_known {
        Err(())
    } else {
        Ok(None)
    }
}

fn token_from(req: &Req) -> Option<&str> {
    let value = req.headers.get("authorization")?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn stored_tokens(state: &State) -> Vec<Value> {
    let text = state
        .manager
        .setting(TOKENS_KEY)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    serde_json::from_str::<Vec<Value>>(&text).unwrap_or_default()
}

async fn scope_of(state: &State, token: &str) -> Scope {
    if token.is_empty() {
        return Scope::None;
    }
    if same_token(token, &state.token) {
        return Scope::Admin;
    }
    let wanted = hash(token);
    let mut found = Scope::None;
    // Every stored token is compared, so the time does not say which one matched.
    for record in stored_tokens(state).await {
        let stored = record.get("hash").and_then(Value::as_str).unwrap_or("");
        let scope = record
            .get("scope")
            .and_then(Value::as_str)
            .and_then(Scope::parse)
            .unwrap_or(Scope::None);
        if same_token(stored, &wanted) {
            found = scope;
        }
    }
    found
}

type Params = HashMap<&'static str, String>;

/// The safety checks and the route. Nothing of the body has been looked at yet.
async fn prepare(
    state: &State,
    req: &Req,
    port: u16,
) -> Result<(&'static Route, Scope, Params), Reply> {
    if !req.remote && !req.headers.get("host").is_some_and(|h| host_ok(h, port)) {
        return Err(failure(
            421,
            "bad_host",
            "the Host header must be 127.0.0.1, localhost or [::1] with this port",
        ));
    }
    // The companion's own page is same-origin; anything else from a web page is refused.
    let same_origin = req.remote
        && match (req.headers.get("origin"), req.headers.get("host")) {
            (None, _) => true,
            (Some(o), Some(h)) => o == &format!("http://{h}"),
            _ => false,
        }
        && req
            .headers
            .get("sec-fetch-site")
            .is_none_or(|v| matches!(v.as_str(), "same-origin" | "none"));
    if !same_origin
        && (req.headers.contains_key("origin")
            || req
                .headers
                .get("sec-fetch-site")
                .is_some_and(|v| v != "none"))
    {
        return Err(failure(
            403,
            "web_pages_not_allowed",
            "this API does not take requests from web pages",
        ));
    }
    let (route, params) = match match_route(&req.method, &req.path) {
        Ok(Some(found)) => found,
        Ok(None) => return Err(failure(404, "not_found", "there is no such address")),
        Err(()) => {
            return Err(failure(
                405,
                "method_not_allowed",
                "this address does not take that method",
            ))
        }
    };
    let scope = if route.scope == Scope::None {
        Scope::None
    } else {
        let given = token_from(req).unwrap_or("");
        if req.remote && same_token(given, &state.token) {
            tokio::time::sleep(Duration::from_millis(250)).await;
            return Err(failure(
                403,
                "master_token_local_only",
                "the master token works only on this computer; make a smaller one with `mohona remote token`",
            ));
        }
        let mut have = scope_of(state, given).await;
        if req.remote {
            have = have.min(Scope::Write); // administration stays on this computer
        }
        if have == Scope::None {
            tokio::time::sleep(Duration::from_millis(250)).await; // slows guessing
            return Err(failure(
                401,
                "unauthorized",
                "send the token as 'Authorization: Bearer <token>'",
            ));
        }
        if have < route.scope {
            return Err(failure(
                403,
                "forbidden_scope",
                format!("this needs a token with the {:?} scope", route.scope).to_lowercase(),
            ));
        }
        have
    };
    if matches!(req.method.as_str(), "POST" | "PUT" | "PATCH") && !req.body.is_empty() {
        let kind = req
            .headers
            .get("content-type")
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_default();
        if !kind.starts_with("application/json") {
            return Err(failure(
                415,
                "json_only",
                "send the body as application/json",
            ));
        }
    }
    Ok((route, scope, params))
}

fn id_of(params: &Params) -> Result<i64, Reply> {
    params
        .get("id")
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| failure(400, "bad_id", "the number in the address is not a valid id"))
}

/// The body as an object ({} when empty).
fn object(req: &Req) -> Result<Map<String, Value>, Reply> {
    if req.body.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(&req.body) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(failure(400, "bad_body", "the body must be a JSON object")),
        Err(e) => Err(failure(
            400,
            "bad_body",
            format!("the body is not valid JSON: {e}"),
        )),
    }
}

/// A call from its type name and fields; the fields are the daemon's own.
fn call(kind: &str, mut fields: Map<String, Value>) -> Result<Call, Reply> {
    fields.insert("type".into(), Value::String(kind.into()));
    serde_json::from_value::<Call>(Value::Object(fields))
        .map_err(|e| failure(400, "bad_body", e.to_string()))
}

async fn ask(state: &State, call: Call) -> Reply {
    let response = handle(state, call).await;
    let mut body = serde_json::to_value(&response).unwrap_or(Value::Null);
    match response {
        Response::Error { message } => {
            let status = if message.contains("no download with id") || message.contains("no such") {
                404
            } else {
                400
            };
            failure(
                status,
                if status == 404 {
                    "not_found"
                } else {
                    "refused"
                },
                message,
            )
        }
        Response::Skipped { reason, .. } => failure(409, "duplicate", reason),
        Response::Done => reply(200, json!({ "ok": true })),
        other => {
            if let Value::Object(map) = &mut body {
                map.remove("type");
            }
            reply(
                if matches!(other, Response::Added { .. }) {
                    201
                } else {
                    200
                },
                body,
            )
        }
    }
}

async fn run(
    state: &State,
    route: &'static Route,
    scope: Scope,
    params: Params,
    req: Req,
) -> Reply {
    match dispatch(state, route, scope, params, req).await {
        Ok(reply) | Err(reply) => reply,
    }
}

async fn dispatch(
    state: &State,
    route: &'static Route,
    _scope: Scope,
    params: Params,
    req: Req,
) -> Result<Reply, Reply> {
    let mut fields = Map::new();
    Ok(match route.name {
        "ping" => reply(
            200,
            json!({ "name": "mohona", "version": env!("CARGO_PKG_VERSION"), "api": 1 }),
        ),
        "openapi" => reply(200, serde_json::from_str(OPENAPI).unwrap_or(Value::Null)),
        "listDownloads" => ask(state, Call::List).await,
        "getDownload" => {
            let id = id_of(&params)?;
            let listed = ask(state, Call::List).await;
            match listed
                .body
                .get("downloads")
                .and_then(Value::as_array)
                .and_then(|d| d.iter().find(|d| d["id"] == id))
            {
                Some(found) => reply(200, found.clone()),
                None if listed.status == 200 => {
                    failure(404, "not_found", format!("no download with id {id}"))
                }
                None => listed,
            }
        }
        "addDownload" => {
            let mut body = object(&req)?;
            if let Some(folder) = body.remove("folder") {
                body.entry("dest_dir").or_insert(folder);
            }
            if req.remote {
                // A phone may not choose where on this computer a file is written.
                body.remove("dest_dir");
                body.remove("cookies");
            }
            ask(state, call("add", body)?).await
        }
        "removeDownload" => {
            let id = id_of(&params)?;
            fields.insert("id".into(), id.into());
            fields.insert(
                "delete_file".into(),
                Value::Bool(
                    !req.remote && req.query.get("delete_file").is_some_and(|v| v == "true"),
                ),
            );
            ask(state, call("remove", fields)?).await
        }
        "downloadDetails" | "pauseDownload" | "resumeDownload" | "cancelDownload" => {
            fields.insert("id".into(), id_of(&params)?.into());
            let kind = match route.name {
                "downloadDetails" => "get_details",
                "pauseDownload" => "pause",
                "resumeDownload" => "resume",
                _ => "cancel",
            };
            ask(state, call(kind, fields)?).await
        }
        "getConfig" => ask(state, Call::GetConfig).await,
        "setConfig" => ask(state, call("set_config", object(&req)?)?).await,
        "listQueues" => ask(state, Call::GetQueues).await,
        "addQueue" => ask(state, call("add_queue", object(&req)?)?).await,
        "setQueue" => {
            let id = id_of(&params)?;
            let mut body = object(&req)?;
            if let Some(name) = body.remove("name") {
                let mut rename = Map::new();
                rename.insert("id".into(), id.into());
                rename.insert("name".into(), name);
                let renamed = ask(state, call("rename_queue", rename)?).await;
                if renamed.status >= 400 {
                    return Ok(renamed);
                }
            }
            body.insert("id".into(), id.into());
            ask(state, call("set_queue", body)?).await
        }
        "removeQueue" => {
            fields.insert("id".into(), id_of(&params)?.into());
            ask(state, call("remove_queue", fields)?).await
        }
        "listCategories" => ask(state, Call::GetCategories).await,
        "addCategory" => ask(state, call("add_category", object(&req)?)?).await,
        "removeCategory" => {
            fields.insert("id".into(), id_of(&params)?.into());
            ask(state, call("remove_category", fields)?).await
        }
        "resolveLink" => {
            let body = object(&req)?;
            let Some(url) = body.get("url").and_then(Value::as_str) else {
                return Err(failure(400, "bad_body", "url: the address to look at"));
            };
            match crate::cloud::resolve(url) {
                Some(link) => reply(200, json!({ "url": link.url, "service": link.service })),
                None => reply(200, json!({ "url": null, "service": null })),
            }
        }
        "torrentFiles" => ask(state, call("torrent_files", object(&req)?)?).await,
        "grabStart" => ask(state, call("grab_start", object(&req)?)?).await,
        "grabStatus" => ask(state, Call::GrabStatus).await,
        "grabStop" => ask(state, Call::GrabStop).await,
        "getRules" => ask(state, Call::GetRules).await,
        "setRules" => {
            let saved = ask(state, call("set_rules", object(&req)?)?).await;
            if saved.status >= 400 {
                saved
            } else {
                ask(state, Call::GetRules).await
            }
        }
        "testRules" => ask(state, call("test_rules", object(&req)?)?).await,
        "getProfiles" => ask(state, Call::GetProfiles).await,
        "setProfiles" => {
            let saved = ask(state, call("set_profiles", object(&req)?)?).await;
            if saved.status >= 400 {
                saved
            } else {
                ask(state, Call::GetProfiles).await
            }
        }
        "getPlan" => ask(state, Call::GetPlan).await,
        "setPlan" => ask(state, call("set_plan", object(&req)?)?).await,
        "setAfterAction" => ask(state, call("set_after_action", object(&req)?)?).await,
        "cancelAfterAction" => ask(state, Call::CancelAction).await,
        "listTokens" => reply(
            200,
            json!({ "tokens": stored_tokens(state).await.into_iter().map(public_token).collect::<Vec<_>>() }),
        ),
        "createToken" => create_token(state, object(&req)?).await,
        "deleteToken" => delete_token(state, id_of(&params)?).await,
        _ => failure(404, "not_found", "there is no such address"),
    })
}

/// A stored token without its hash.
fn public_token(record: Value) -> Value {
    json!({ "id": record["id"], "name": record["name"], "scope": record["scope"], "created": record["created"] })
}

/// A new token for the CLI (the companion's phone login). Only read and write.
pub(crate) async fn make_token(
    state: &State,
    name: String,
    scope: String,
) -> Result<(i64, String), String> {
    if scope != "read" && scope != "write" {
        return Err("scope must be read or write".into());
    }
    let mut body = Map::new();
    body.insert("name".into(), Value::String(name));
    body.insert("scope".into(), Value::String(scope));
    let made = create_token(state, body).await;
    if made.status != 201 {
        return Err(made.body["error"]["message"]
            .as_str()
            .unwrap_or("could not make the token")
            .to_string());
    }
    Ok((
        made.body["id"].as_i64().unwrap_or(0),
        made.body["token"].as_str().unwrap_or("").to_string(),
    ))
}

async fn create_token(state: &State, body: Map<String, Value>) -> Reply {
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return failure(400, "bad_body", "name: 1 to 40 characters");
    }
    let scope = body.get("scope").and_then(Value::as_str).unwrap_or("read");
    if Scope::parse(scope).is_none() {
        return failure(400, "bad_body", "scope must be read, write or admin");
    }
    let mut tokens = stored_tokens(state).await;
    if tokens.len() >= MAX_TOKENS {
        return failure(
            400,
            "too_many_tokens",
            format!("at most {MAX_TOKENS} tokens"),
        );
    }
    let id = tokens
        .iter()
        .filter_map(|t| t["id"].as_i64())
        .max()
        .unwrap_or(0)
        + 1;
    let secret = format!("mhn_{}", crate::DaemonInfo::new_token());
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    tokens.push(json!({ "id": id, "name": name, "scope": scope, "hash": hash(&secret), "created": created }));
    match state
        .manager
        .set_setting(TOKENS_KEY, &Value::Array(tokens).to_string())
        .await
    {
        Ok(()) => reply(
            201,
            json!({ "id": id, "name": name, "scope": scope, "created": created, "token": secret }),
        ),
        Err(e) => failure(400, "refused", e.to_string()),
    }
}

async fn delete_token(state: &State, id: i64) -> Reply {
    let mut tokens = stored_tokens(state).await;
    let before = tokens.len();
    tokens.retain(|t| t["id"].as_i64() != Some(id));
    if tokens.len() == before {
        return failure(404, "not_found", format!("no token with id {id}"));
    }
    match state
        .manager
        .set_setting(TOKENS_KEY, &Value::Array(tokens).to_string())
        .await
    {
        Ok(()) => reply(200, json!({ "ok": true })),
        Err(e) => failure(400, "refused", e.to_string()),
    }
}

// --- events (WebSocket) ----------------------------------------------------

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The `Sec-WebSocket-Accept` value for a client's key.
pub fn accept_key(key: &str) -> String {
    let mut sha = Sha1::new();
    sha.update(key.as_bytes());
    sha.update(GUID.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(sha.finalize())
}

fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => out.push(n as u8),
        n if n <= 0xFFFF => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    out
}

/// What a client sees of a manager event (None: not worth sending).
fn event_json(event: &ManagerEvent) -> Option<Value> {
    Some(match event {
        ManagerEvent::Added { id } => json!({ "type": "added", "id": id }),
        ManagerEvent::State { id, status } => {
            json!({ "type": "state", "id": id, "status": status.as_str() })
        }
        ManagerEvent::Progress {
            id,
            downloaded,
            total,
        } => json!({ "type": "progress", "id": id, "downloaded": downloaded, "total": total }),
        ManagerEvent::Error { id, message } => {
            json!({ "type": "error", "id": id, "message": message })
        }
        ManagerEvent::Removed { id } => json!({ "type": "removed", "id": id }),
        _ => return None,
    })
}

async fn events(mut stream: TcpStream, state: Arc<State>, req: Req) -> std::io::Result<()> {
    let upgrade = req
        .headers
        .get("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        && req
            .headers
            .get("connection")
            .is_some_and(|v| v.to_ascii_lowercase().contains("upgrade"))
        && req
            .headers
            .get("sec-websocket-version")
            .is_some_and(|v| v == "13");
    let Some(key) = req.headers.get("sec-websocket-key").filter(|_| upgrade) else {
        return write_reply(
            &mut stream,
            failure(
                400,
                "websocket_only",
                "this address is a WebSocket: send an Upgrade request",
            ),
        )
        .await;
    };
    let head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    );
    stream.write_all(head.as_bytes()).await?;
    let mut feed = state.manager.subscribe();
    let (mut read, mut write) = stream.into_split();
    write
        .write_all(&frame(
            1,
            json!({ "type": "hello", "api": 1 }).to_string().as_bytes(),
        ))
        .await?;
    let mut last_sent: HashMap<i64, Instant> = HashMap::new();
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await;
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        tokio::select! {
            event = feed.recv() => match event {
                Ok(event) => {
                    if let ManagerEvent::Progress { id, downloaded, total } = &event {
                        let done = total.is_some_and(|t| *downloaded >= t);
                        if !done && last_sent.get(id).is_some_and(|at| at.elapsed() < Duration::from_millis(250)) {
                            continue;
                        }
                        last_sent.insert(*id, Instant::now());
                    }
                    if let Some(value) = event_json(&event) {
                        write.write_all(&frame(1, value.to_string().as_bytes())).await?;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return Ok(()),
            },
            _ = ping.tick() => write.write_all(&frame(9, b"")).await?,
            n = read.read(&mut chunk) => {
                let n = n?;
                if n == 0 {
                    return Ok(());
                }
                buffer.extend_from_slice(&chunk[..n]);
                // Clients may only close, ping or pong; nothing else is read.
                while let Some((opcode, used)) = client_frame(&buffer)? {
                    buffer.drain(..used);
                    match opcode {
                        8 => {
                            let _ = write.write_all(&frame(8, b"")).await;
                            return Ok(());
                        }
                        9 => write.write_all(&frame(10, b"")).await?,
                        _ => {}
                    }
                }
                if buffer.len() > 4096 {
                    return Ok(());
                }
            }
        }
    }
}

/// The first complete client frame in `data`: its opcode and its length.
/// Client frames must be masked and small; anything else ends the connection.
fn client_frame(data: &[u8]) -> std::io::Result<Option<(u8, usize)>> {
    let bad = || std::io::Error::other("bad WebSocket frame");
    if data.len() < 2 {
        return Ok(None);
    }
    let opcode = data[0] & 0x0F;
    if data[1] & 0x80 == 0 {
        return Err(bad());
    }
    let (length, header) = match data[1] & 0x7F {
        126 => {
            if data.len() < 4 {
                return Ok(None);
            }
            (u16::from_be_bytes([data[2], data[3]]) as usize, 4)
        }
        127 => return Err(bad()),
        n => (n as usize, 2),
    };
    if length > 1024 {
        return Err(bad());
    }
    let total = header + 4 + length;
    if data.len() < total {
        return Ok(None);
    }
    Ok(Some((opcode, total)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_must_name_this_server_and_port() {
        assert!(host_ok("127.0.0.1:6791", 6791));
        assert!(host_ok("localhost:6791", 6791));
        assert!(host_ok("LOCALHOST:6791", 6791));
        assert!(host_ok("[::1]:6791", 6791));
        for bad in [
            "127.0.0.1",
            "127.0.0.1:80",
            "evil.example:6791",
            "127.0.0.1.evil.example:6791",
            "localhost.evil.com:6791",
            "",
        ] {
            assert!(!host_ok(bad, 6791), "{bad}");
        }
    }

    #[test]
    fn websocket_key_matches_the_specification_example() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn routes_match_with_ids_and_report_wrong_methods() {
        let (route, params) = match_route("GET", "/v1/downloads/7/details")
            .unwrap()
            .unwrap();
        assert_eq!(route.name, "downloadDetails");
        assert_eq!(params["id"], "7");
        assert!(match_route("PATCH", "/v1/downloads").is_err());
        assert!(match_route("GET", "/v2/downloads").unwrap().is_none());
        assert!(match_route("GET", "/v1/downloads/").unwrap().is_some());
    }

    #[test]
    fn frames_are_built_and_read_back() {
        assert_eq!(frame(1, b"hi"), vec![0x81, 2, b'h', b'i']);
        assert_eq!(frame(1, &[0u8; 200])[..4], [0x81, 126, 0, 200]);
        let masked = [0x88, 0x80, 1, 2, 3, 4];
        assert_eq!(client_frame(&masked).unwrap(), Some((8, 6)));
        assert!(client_frame(&[0x81, 0x81]).unwrap().is_none());
        assert!(client_frame(&[0x81, 0x02, 0, 0, 0, 0]).is_err()); // unmasked
    }

    #[test]
    fn tokens_are_kept_hashed() {
        assert_eq!(
            hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
