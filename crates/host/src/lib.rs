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

    fn add(&self, message: &mut Value) -> Value {
        let Some(map) = message.as_object_mut() else {
            return error("bad_request", "the message is not an object");
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
        let url = match text(map, "url") {
            Ok(Some(url)) => url,
            Ok(None) => return error("bad_request", "\"url\" is missing"),
            Err(e) => return e,
        };
        let (bare, _) = mohona_net::split_userinfo(&url);
        let scheme_ok = ["http://", "https://", "ftp://", "ftps://", "sftp://"]
            .iter()
            .any(|s| bare.to_ascii_lowercase().starts_with(s));
        if !scheme_ok {
            return error(
                "bad_url",
                "only http, https, ftp, ftps and sftp addresses can be downloaded",
            );
        }
        let (mut referrer, mut cookies, mut user_agent) = (None, None, None);
        for (key, slot) in [
            ("referrer", &mut referrer),
            ("cookies", &mut cookies),
            ("userAgent", &mut user_agent),
        ] {
            match text(map, key) {
                Ok(v) => *slot = v,
                Err(e) => return e,
            }
        }
        let mut headers = Vec::new();
        if let Some(list) = map.get("headers") {
            let Some(list) = list.as_array() else {
                return error("bad_request", "\"headers\" must be a list");
            };
            for header in list {
                let (Some(name), Some(value)) = (
                    header.get("name").and_then(Value::as_str),
                    header.get("value").and_then(Value::as_str),
                ) else {
                    return error(
                        "bad_request",
                        "each header needs a \"name\" and a \"value\"",
                    );
                };
                if name.chars().any(char::is_control) || value.chars().any(char::is_control) {
                    return error("bad_request", "a header contains control characters");
                }
                headers.push((name.to_string(), value.to_string()));
            }
        }
        let answer = self.call(Call::Add {
            url: url.trim().to_string(),
            dest_dir: None,
            cookies,
            referrer,
            user_agent,
            headers,
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
