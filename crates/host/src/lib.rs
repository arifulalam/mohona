//! The native messaging host.
//!
//! A browser starts this program when the Mohona extension asks to talk to
//! it, and speaks to it over standard input and output: each message is a
//! 4-byte length (in the machine's byte order) followed by that many bytes
//! of UTF-8 JSON. Everything the host writes to standard output must be such
//! a message; anything else would corrupt the conversation, so diagnostics
//! go to standard error.
//!
//! The host never downloads anything itself. It hands the download to the
//! Mohona engine (`mohona daemon`) through its local API and tells the
//! extension what happened. See [`Host::handle`] for the messages.

pub mod register;

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mohona_daemon::{Call, Client, ClientError, Response};
use serde_json::{json, Map, Value};

/// Version of the message format. The extension checks it in `hello`.
pub const PROTOCOL: u32 = 1;

/// The largest message accepted from the browser.
pub const MAX_MESSAGE: usize = 1024 * 1024;

/// Reads one message. `Ok(None)` is a clean end of input (the browser
/// closed the pipe).
pub fn read_message(input: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0u8; 4];
    match input.read_exact(&mut length) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let length = u32::from_ne_bytes(length) as usize;
    if length > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message of {length} bytes is larger than the {MAX_MESSAGE} byte limit"),
        ));
    }
    let mut body = vec![0u8; length];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Writes one message and flushes it.
pub fn write_message(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    output.write_all(&(body.len() as u32).to_ne_bytes())?;
    output.write_all(&body)?;
    output.flush()
}

fn error(code: &str, message: impl Into<String>) -> Value {
    json!({"type": "error", "code": code, "message": message.into()})
}

/// How long `startEngine` waits for a freshly started engine to answer.
const START_WAIT: Duration = Duration::from_secs(8);

/// The fields of a download message, checked.
struct Details {
    url: String,
    referrer: Option<String>,
    cookies: Option<String>,
    user_agent: Option<String>,
    headers: Vec<(String, String)>,
    name: Option<String>,
}

pub struct Host {
    /// The engine's connection file (see `mohona_daemon::DaemonInfo`).
    info_path: PathBuf,
    /// The database the engine should use when the host starts it.
    database: Option<PathBuf>,
    /// The `mohona` program, for `startEngine`.
    engine_program: Option<PathBuf>,
    runtime: tokio::runtime::Runtime,
}

impl Host {
    pub fn new(
        info_path: PathBuf,
        database: Option<PathBuf>,
        engine_program: Option<PathBuf>,
    ) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for the host");
        Self {
            info_path,
            database,
            engine_program,
            runtime,
        }
    }

    /// One call to the engine. `Err` is a ready-made error answer.
    fn call(&self, call: Call) -> Result<Response, Value> {
        let client = Client::from_info_file(&self.info_path).map_err(engine_error)?;
        self.runtime
            .block_on(client.call(call))
            .map_err(engine_error)
    }

    fn ping(&self) -> Option<(String, u32)> {
        match self.call(Call::Ping) {
            Ok(Response::Pong { version, pid }) => Some((version, pid)),
            _ => None,
        }
    }

    /// Answers one message. The answer carries the `requestId` of the
    /// question, so the extension can match them.
    ///
    /// | `type` | meaning |
    /// |---|---|
    /// | `hello` | who is there: `protocol`, `version`, `engineRunning`, `engineVersion` |
    /// | `add` | put `url` in the list, with optional `referrer`, `cookies`, `userAgent`, `headers` |
    /// | `stream` | download the HLS/DASH stream at `url` (same optional fields, and `name`) |
    /// | `saveStart`, `savePiece`, `saveEnd`, `saveAbort` | save a file the browser holds (`blob:`, `data:`): `name`, `mime`; then pieces `transfer`, `index`, `data` (base64, up to 512 KiB each); then `saveEnd` answers `saved` with `path` |
    /// | `list` | the downloads: `downloads` with `id`, `url`, `status`, `error`, `expired` (the link was refused, 403 or 410) and `page` (where the link was) |
    /// | `refresh` | continue download `id` from the fresh link `url` (optional `cookies`, `referrer`, `userAgent`); refused when the link is not the same file |
    /// | `startEngine` | start `mohona daemon` if it is not answering, and wait for it |
    pub fn handle(&self, message: &[u8]) -> Value {
        let mut value: Value = match serde_json::from_slice(message) {
            Ok(v) => v,
            Err(e) => return error("bad_json", format!("the message is not JSON: {e}")),
        };
        let request_id = value.get("requestId").cloned();
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut answer = match kind.as_deref() {
            Some("hello") => self.hello(),
            Some("add") => self.add(&mut value),
            Some("stream") => self.stream(&mut value),
            Some("list") => self.list(),
            Some("refresh") => self.refresh(&mut value),
            Some("saveStart") => self.save_start(&value),
            Some("savePiece") => self.save_piece(&value),
            Some("saveEnd") => self.save_end(&value),
            Some("saveAbort") => self.save_abort(&value),
            Some("startEngine") => self.start_engine(),
            Some(other) => error("unknown_type", format!("unknown message type \"{other}\"")),
            None => error("bad_request", "the message has no \"type\""),
        };
        if let (Some(id), Some(map)) = (request_id, answer.as_object_mut()) {
            map.insert("requestId".into(), id);
        }
        answer
    }

    fn hello(&self) -> Value {
        let engine = self.ping();
        json!({
            "type": "hello",
            "protocol": PROTOCOL,
            "version": env!("CARGO_PKG_VERSION"),
            "engineRunning": engine.is_some(),
            "engineVersion": engine.map(|(version, _)| version),
        })
    }

    /// Checks the fields every download message shares. `Err` is the answer to give.
    fn details(message: &mut Value, schemes: &[&str]) -> Result<Details, Value> {
        let Some(map) = message.as_object_mut() else {
            return Err(error("bad_request", "the message is not an object"));
        };
        let text = |map: &Map<String, Value>, key: &str| -> Result<Option<String>, Value> {
            match map.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if s.chars().any(char::is_control) => Err(error(
                    "bad_request",
                    format!("\"{key}\" contains control characters"),
                )),
                Some(Value::String(s)) if s.is_empty() => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(error("bad_request", format!("\"{key}\" must be text"))),
            }
        };
        let Some(url) = text(map, "url")? else {
            return Err(error("bad_request", "\"url\" is missing"));
        };
        let (bare, _) = mohona_net::split_userinfo(&url);
        let lower = bare.to_ascii_lowercase();
        if !schemes
            .iter()
            .any(|s| lower.starts_with(&format!("{s}://")))
        {
            return Err(error(
                "bad_url",
                format!(
                    "only {} addresses can be downloaded here",
                    schemes.join(", ")
                ),
            ));
        }
        let mut headers = Vec::new();
        if let Some(list) = map.get("headers") {
            let Some(list) = list.as_array() else {
                return Err(error("bad_request", "\"headers\" must be a list"));
            };
            for header in list {
                let (Some(name), Some(value)) = (
                    header.get("name").and_then(Value::as_str),
                    header.get("value").and_then(Value::as_str),
                ) else {
                    return Err(error(
                        "bad_request",
                        "each header needs a \"name\" and a \"value\"",
                    ));
                };
                if name.chars().any(char::is_control) || value.chars().any(char::is_control) {
                    return Err(error("bad_request", "a header contains control characters"));
                }
                headers.push((name.to_string(), value.to_string()));
            }
        }
        let name = text(map, "name")?;
        if name
            .as_deref()
            .is_some_and(|n| n.contains(['/', '\\', ':']) || n.starts_with('.'))
        {
            return Err(error("bad_request", "\"name\" must be a plain file name"));
        }
        Ok(Details {
            url: url.trim().to_string(),
            referrer: text(map, "referrer")?,
            cookies: text(map, "cookies")?,
            user_agent: text(map, "userAgent")?,
            headers,
            name,
        })
    }

    fn add(&self, message: &mut Value) -> Value {
        let d = match Self::details(message, &["http", "https", "ftp", "ftps", "sftp"]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let answer = self.call(Call::Add {
            url: d.url,
            dest_dir: None,
            cookies: d.cookies,
            referrer: d.referrer,
            user_agent: d.user_agent,
            headers: d.headers,
            sort: true,
            duplicates: Some("skip".into()),
        });
        match answer {
            Ok(Response::Added { id }) => json!({"type": "added", "id": id}),
            Ok(Response::Skipped { reason }) => json!({"type": "duplicate", "message": reason}),
            Ok(Response::Error { message }) => error("engine", message),
            Ok(_) => error("engine", "the engine gave an unexpected answer"),
            Err(e) => e,
        }
    }

    fn list(&self) -> Value {
        match self.call(Call::List) {
            Ok(Response::Downloads { downloads }) => {
                let rows: Vec<Value> = downloads
                    .into_iter()
                    .map(|d| {
                        json!({
                            "id": d.id, "url": d.url, "filename": d.filename,
                            "status": d.status, "downloaded": d.downloaded, "size": d.size,
                            "error": d.error, "path": d.path,
                            "page": d.page, "expired": d.expired,
                        })
                    })
                    .collect();
                json!({"type": "downloads", "downloads": rows})
            }
            Ok(Response::Error { message }) => error("engine", message),
            Ok(_) => error("engine", "the engine gave an unexpected answer"),
            Err(e) => e,
        }
    }

    fn refresh(&self, message: &mut Value) -> Value {
        let id = match Self::number(message, "id") {
            Ok(id) => id as i64,
            Err(e) => return e,
        };
        let d = match Self::details(message, &["http", "https"]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        match self.call(Call::Refresh {
            id,
            url: d.url,
            cookies: d.cookies,
            referrer: d.referrer,
            user_agent: d.user_agent,
        }) {
            Ok(Response::Done) => json!({"type": "refreshed", "id": id}),
            Ok(Response::Error { message }) => error("refused", message),
            Ok(_) => error("engine", "the engine gave an unexpected answer"),
            Err(e) => e,
        }
    }

    fn number(value: &Value, key: &str) -> Result<u64, Value> {
        value
            .get(key)
            .and_then(Value::as_u64)
            .ok_or_else(|| error("bad_request", format!("\"{key}\" must be a number")))
    }

    fn save_start(&self, value: &Value) -> Value {
        let text = |key: &str| value.get(key).and_then(Value::as_str);
        let name = text("name").unwrap_or("download");
        let mime = text("mime");
        if [Some(name), mime]
            .into_iter()
            .flatten()
            .any(|t| t.chars().any(char::is_control))
        {
            return error("bad_request", "a field contains control characters");
        }
        match self.call(Call::ReceiveStart {
            name: name.to_string(),
            mime: mime.map(str::to_string),
            dest_dir: None,
        }) {
            Ok(Response::Receiving { transfer }) => json!({"type": "saving", "transfer": transfer}),
            Ok(Response::Error { message }) => error("engine", message),
            Ok(_) => error("engine", "the engine gave an unexpected answer"),
            Err(e) => e,
        }
    }

    fn save_piece(&self, value: &Value) -> Value {
        let (transfer, index) = match (
            Self::number(value, "transfer"),
            Self::number(value, "index"),
        ) {
            (Ok(t), Ok(i)) => (t, i),
            (Err(e), _) | (_, Err(e)) => return e,
        };
        let Some(data) = value.get("data").and_then(Value::as_str) else {
            return error("bad_request", "\"data\" must be text");
        };
        match self.call(Call::ReceivePiece {
            transfer,
            index,
            data: data.to_string(),
        }) {
            Ok(Response::Done) => json!({"type": "piece", "index": index}),
            Ok(Response::Error { message }) => error("engine", message),
            Ok(_) => error("engine", "the engine gave an unexpected answer"),
            Err(e) => e,
        }
    }

    fn save_end(&self, value: &Value) -> Value {
        let transfer = match Self::number(value, "transfer") {
            Ok(t) => t,
            Err(e) => return e,
        };
        match self.call(Call::ReceiveEnd { transfer }) {
            Ok(Response::Saved { path, name }) => {
                json!({"type": "saved", "path": path, "name": name})
            }
            Ok(Response::Error { message }) => error("engine", message),
            Ok(_) => error("engine", "the engine gave an unexpected answer"),
            Err(e) => e,
        }
    }

    fn save_abort(&self, value: &Value) -> Value {
        let transfer = match Self::number(value, "transfer") {
            Ok(t) => t,
            Err(e) => return e,
        };
        match self.call(Call::ReceiveAbort { transfer }) {
            Ok(_) => json!({"type": "aborted"}),
            Err(e) => e,
        }
    }

    /// An HLS or DASH stream: the engine runs `mohona stream` for it.
    fn stream(&self, message: &mut Value) -> Value {
        let d = match Self::details(message, &["http", "https"]) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let answer = self.call(Call::Stream {
            url: d.url,
            dest_dir: None,
            cookies: d.cookies,
            referrer: d.referrer,
            user_agent: d.user_agent,
            headers: d.headers,
            name: d.name,
        });
        match answer {
            Ok(Response::Started { job, folder }) => {
                json!({"type": "streamStarted", "job": job, "folder": folder})
            }
            Ok(Response::Error { message }) => error("engine", message),
            Ok(_) => error("engine", "the engine gave an unexpected answer"),
            Err(e) => e,
        }
    }

    fn start_engine(&self) -> Value {
        if self.ping().is_some() {
            return json!({"type": "engine", "running": true, "started": false});
        }
        let Some(program) = &self.engine_program else {
            return error(
                "no_engine",
                "the mohona program was not found next to the host",
            );
        };
        if let Err(e) = spawn_engine(program, &self.info_path, self.database.as_deref()) {
            return error("no_engine", format!("could not start Mohona: {e}"));
        }
        let started = Instant::now();
        while started.elapsed() < START_WAIT {
            if self.ping().is_some() {
                return json!({"type": "engine", "running": false, "started": true});
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        error("no_engine", "Mohona was started but did not answer in time")
    }
}

fn engine_error(e: ClientError) -> Value {
    match e {
        ClientError::NotRunning => error(
            "engine_not_running",
            "Mohona is not running; send startEngine first",
        ),
        ClientError::Protocol(m) => error("engine", m),
    }
}

/// Starts `mohona daemon` in the background, not tied to the browser.
fn spawn_engine(program: &Path, info: &Path, database: Option<&Path>) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    if let Some(database) = database {
        command.arg("--db").arg(database);
    }
    command
        .arg("daemon")
        .arg("--info")
        .arg(info)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window, and not part of the browser's process group.
        command.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    command.spawn().map(|_| ())
}

/// Serves messages until the browser closes the pipe.
pub fn serve(host: &Host, input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    while let Some(message) = read_message(input)? {
        let answer = host.handle(&message);
        write_message(output, &answer)?;
    }
    Ok(())
}
