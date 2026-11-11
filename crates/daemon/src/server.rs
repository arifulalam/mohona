use std::path::PathBuf;
use std::sync::Arc;

use mohona_manager::{
    AddRequest, AddResult, AfterAction, DuplicatePolicy, Manager, ManagerError, ManagerEvent,
    Moment, Schedule, SpeedTarget,
};
use mohona_net::RequestContext;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;

use crate::info::DaemonInfo;
use crate::protocol::{
    Call, CategoryInfo, ConnectionInfo, DownloadInfo, QueueInfo, Request, Response,
};
use crate::streams::StreamJob;

/// The longest request line accepted.
const MAX_LINE: usize = 1024 * 1024;

pub struct ServerConfig {
    /// Where to write [`DaemonInfo`] (removed again on exit).
    pub info_path: PathBuf,
    /// Where downloads go unless the client says otherwise.
    pub default_dir: PathBuf,
    /// The `mohona` program that runs stream downloads (`mohona stream`).
    /// `None`: stream calls are refused.
    pub stream_program: Option<PathBuf>,
    /// Pass a [`Notify`] to stop the daemon from outside (Ctrl-C); a
    /// `shutdown` call stops it too.
    pub shutdown: Arc<Notify>,
}

pub(crate) struct State {
    pub(crate) manager: Manager,
    pub(crate) token: String,
    default_dir: PathBuf,
    shutdown: Arc<Notify>,
    streams: crate::streams::Streams,
    receiver: crate::receive::Receiver,
    action: std::sync::Mutex<ActionState>,
    live: std::sync::Mutex<std::collections::HashMap<i64, Live>>,
    /// The port the remote companion listens on (0: not listening).
    remote_port: std::sync::atomic::AtomicU16,
    plugins_dir: PathBuf,
    grab: Arc<std::sync::Mutex<Option<GrabJob>>>,
}

/// The site grabber's run: the newest one, running or over.
struct GrabJob {
    cancel: mohona_core::CancelToken,
    progress: mohona_grabber::Progress,
    state: &'static str,
    result: Option<mohona_grabber::Grabbed>,
    error: Option<String>,
}

/// The chosen repository: the default until one is saved, and nothing when it was emptied.
async fn update_repo(manager: &Manager) -> String {
    manager
        .setting(crate::update::REPO_SETTING)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| crate::update::DEFAULT_REPO.to_string())
}

async fn update_setting(manager: &Manager, found: Option<crate::update::Release>) -> Response {
    let repo = update_repo(manager).await;
    let current = env!("CARGO_PKG_VERSION").to_string();
    Response::Update {
        newer: found
            .as_ref()
            .is_some_and(|r| crate::update::is_newer(&current, &r.version)),
        latest: found.as_ref().map(|r| r.version.clone()),
        url: found.as_ref().map(|r| r.url.clone()),
        notes: found.map(|r| r.notes),
        repo,
        current,
    }
}

async fn scan_setting(manager: &Manager) -> Response {
    let get = |key: &'static str| async move {
        manager
            .setting(key)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    };
    let mode = get(mohona_manager::scan::MODE).await;
    Response::Scan {
        mode: if mode.is_empty() { "off".into() } else { mode },
        command: get(mohona_manager::scan::COMMAND).await,
        defender: mohona_manager::scan::defender().map(|p| p.display().to_string()),
    }
}

/// The newest numbers about a running download, from the manager's events.
struct Live {
    downloaded: u64,
    parts: Vec<LivePart>,
    at: std::time::Instant,
    /// Bytes per second over the last seconds, all connections.
    speed: f64,
    seen: (std::time::Instant, u64),
}

struct LivePart {
    start: u64,
    end: u64,
    position: u64,
    speed: f64,
    moved_at: std::time::Instant,
    sampled: std::time::Instant,
}

fn tool_named(name: &str) -> Option<mohona_media::tools::Tool> {
    [mohona_media::tools::FFMPEG, mohona_media::tools::YTDLP]
        .into_iter()
        .find(|t| t.name == name)
}

fn tool_key(tool: &mohona_media::tools::Tool) -> String {
    format!("tools.{}", tool.program)
}

async fn chosen_tool(state: &State, tool: &mohona_media::tools::Tool) -> Option<String> {
    state
        .manager
        .setting(&tool_key(tool))
        .await
        .ok()
        .flatten()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
}

async fn tool_list(state: &State) -> Vec<crate::protocol::ToolStatus> {
    let mut out = Vec::new();
    for tool in [mohona_media::tools::FFMPEG, mohona_media::tools::YTDLP] {
        let chosen = chosen_tool(state, &tool).await;
        let found = tool
            .find(chosen.as_deref().map(std::path::Path::new))
            .map(|p| p.display().to_string());
        out.push(crate::protocol::ToolStatus {
            name: tool.name.into(),
            purpose: tool.purpose.into(),
            chosen,
            found,
            page: tool.page.into(),
            windows_page: tool.windows.into(),
            winget: tool.winget.into(),
        });
    }
    out
}

async fn enabled_plugins(state: &State) -> Vec<String> {
    state
        .manager
        .setting(crate::plugins::ENABLED_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

async fn plugin_list(state: &State) -> Vec<crate::plugins::PluginInfo> {
    let on = enabled_plugins(state).await;
    crate::plugins::discover(&state.plugins_dir)
        .into_iter()
        .map(|(label, found)| match found {
            Ok(p) => crate::plugins::PluginInfo {
                enabled: on.contains(&p.name),
                name: p.name,
                version: p.version,
                description: p.description,
                hosts: p.hosts,
                problem: None,
            },
            Err(why) => crate::plugins::PluginInfo {
                name: label,
                version: String::new(),
                description: String::new(),
                hosts: Vec::new(),
                enabled: false,
                problem: Some(why),
            },
        })
        .collect()
}

/// Runs the enabled plugin for `url`'s site, if there is one: its name and what it said.
async fn plugin_for(
    state: &State,
    url: &str,
) -> Option<(String, Result<crate::plugins::Resolved, String>)> {
    let on = enabled_plugins(state).await;
    if on.is_empty() {
        return None;
    }
    let plugins: Vec<_> = crate::plugins::discover(&state.plugins_dir)
        .into_iter()
        .filter_map(|(_, p)| p.ok())
        .collect();
    let plugin = crate::plugins::find(&plugins, &on, url)?;
    Some((
        plugin.name.clone(),
        crate::plugins::resolve(plugin, url).await,
    ))
}

async fn sync_folder(state: &State) -> Option<String> {
    state
        .manager
        .setting(crate::sync::FOLDER_SETTING)
        .await
        .ok()
        .flatten()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
}

/// Merges the portable settings with the file in `folder`. Returns what changed here and what was written.
pub(crate) async fn run_sync(
    state: &State,
    folder: &std::path::Path,
) -> Result<(Vec<String>, Vec<String>), String> {
    use crate::sync as s;
    if !folder.is_dir() {
        return Err(format!("{} is not a folder", folder.display()));
    }
    let path = folder.join(s::FILE_NAME);
    let file = match std::fs::metadata(&path) {
        Ok(m) if m.len() > 1024 * 1024 => return Err("the settings file is too large".into()),
        Ok(_) => {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            Some(
                serde_json::from_str::<s::File>(&text)
                    .map_err(|e| format!("the settings file is damaged: {e}"))?,
            )
        }
        Err(_) => None,
    };
    let mut local = std::collections::BTreeMap::new();
    for key in s::KEYS {
        if let Ok(Some(v)) = state.manager.setting(key).await {
            if !v.is_empty() {
                local.insert((*key).to_string(), v);
            }
        }
    }
    let marks: s::Marks = state
        .manager
        .setting(s::STATE_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let device = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "a computer".into());
    let out = s::merge(&local, &marks, file, now, &device);
    if let Some(file) = &out.file {
        let text = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
        let tmp = folder.join(format!("{}.tmp", s::FILE_NAME));
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    }
    for (key, value) in &out.apply {
        state
            .manager
            .set_setting(key, value)
            .await
            .map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string(&out.marks).map_err(|e| e.to_string())?;
    state
        .manager
        .set_setting(s::STATE_SETTING, &text)
        .await
        .map_err(|e| e.to_string())?;
    Ok((out.apply.into_iter().map(|(k, _)| k).collect(), out.wrote))
}

const REMOTE_KEY: &str = "remote.port";

/// The companion's chosen port: the environment first, then the setting; 0 means off.
async fn remote_setting(state: &State) -> u16 {
    if let Some(p) = std::env::var("MOHONA_REMOTE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
    {
        return p;
    }
    state
        .manager
        .setting(REMOTE_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

/// What the person armed for the end of the downloads, and how far it is.
#[derive(Default)]
struct ActionState {
    armed: Option<String>,
    countdown: Option<u32>,
    message: Option<String>,
}

/// Runs the API until shutdown, then stops the manager and removes the info
/// file. Refuses to start when another daemon answers at the address in the
/// existing info file.
pub async fn serve(manager: Manager, config: ServerConfig) -> std::io::Result<()> {
    if let Ok(old) = DaemonInfo::read(&config.info_path) {
        let alive = crate::Client::new(&old).call(Call::Ping).await.is_ok();
        if alive {
            return Err(std::io::Error::other(format!(
                "a Mohona daemon is already running (process {})",
                old.pid
            )));
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    // The public API: MOHONA_API_PORT picks its port ("off" turns it off).
    let api_listener = match std::env::var("MOHONA_API_PORT").ok().as_deref() {
        Some("off") => None,
        other => crate::api::bind(
            other
                .and_then(|p| p.parse().ok())
                .unwrap_or(crate::api::DEFAULT_PORT),
        )
        .await
        .ok(),
    };
    let info = DaemonInfo {
        api_port: api_listener
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map_or(0, |a| a.port()),
        port: listener.local_addr()?.port(),
        token: DaemonInfo::new_token(),
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    info.write(&config.info_path)?;
    let state = Arc::new(State {
        manager,
        token: info.token.clone(),
        default_dir: config.default_dir,
        shutdown: config.shutdown.clone(),
        streams: crate::streams::Streams::new(config.stream_program),
        receiver: crate::receive::Receiver::default(),
        action: std::sync::Mutex::new(ActionState::default()),
        live: std::sync::Mutex::new(std::collections::HashMap::new()),
        remote_port: std::sync::atomic::AtomicU16::new(0),
        grab: Arc::new(std::sync::Mutex::new(None)),
        plugins_dir: config
            .info_path
            .parent()
            .map_or_else(|| PathBuf::from("plugins"), |p| p.join("plugins")),
    });
    watch_action(&state);
    // The remote companion: off unless MOHONA_REMOTE_PORT or the setting `remote.port` names a port.
    let wanted = remote_setting(&state).await;
    if wanted != 0 {
        if let Ok(listener) = crate::api::bind_remote(wanted).await {
            if let Ok(addr) = listener.local_addr() {
                state
                    .remote_port
                    .store(addr.port(), std::sync::atomic::Ordering::Relaxed);
            }
            tokio::spawn(crate::api::serve_remote(listener, Arc::clone(&state)));
        }
    }
    // Every few minutes, when a synced folder is chosen.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                if let Some(folder) = sync_folder(&state).await {
                    let _ = run_sync(&state, std::path::Path::new(&folder)).await;
                }
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });
    }
    if let Some(api_listener) = api_listener {
        tokio::spawn(crate::api::serve(api_listener, Arc::clone(&state)));
    }
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { continue };
                if !peer.ip().is_loopback() {
                    continue;
                }
                let state = Arc::clone(&state);
                tokio::spawn(async move { let _ = connection(stream, state).await; });
            }
            () = config.shutdown.notified() => break,
        }
    }
    let _ = std::fs::remove_file(&config.info_path);
    state.manager.shutdown().await;
    Ok(())
}

fn check_queue_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(|c| c.is_control()) {
        return Err("a queue's name must be 1 to 40 characters".into());
    }
    Ok(name.to_string())
}

async fn queues(state: &State) -> Response {
    let rows = match state.manager.queues().await {
        Ok(q) => q,
        Err(e) => return error(e),
    };
    let now = Moment::now();
    Response::Queues {
        queues: rows
            .into_iter()
            .map(|q| {
                let schedule = q
                    .schedule_json
                    .as_deref()
                    .and_then(|j| Schedule::from_json(j).ok());
                QueueInfo {
                    id: q.id,
                    name: q.name,
                    max_active: q.max_active,
                    speed_limit: q.speed_limit,
                    open_now: schedule
                        .as_ref()
                        .is_none_or(|s| s.open_window(now).is_some()),
                    windows: schedule.map(|s| s.windows).unwrap_or_default(),
                }
            })
            .collect(),
    }
}

/// Everything is checked before anything changes, so a refused change is never half applied.
async fn set_queue(
    state: &State,
    id: i64,
    max_active: Option<u32>,
    speed_limit: Option<u64>,
    windows: Option<Vec<mohona_manager::Window>>,
) -> Response {
    let manager = &state.manager;
    match manager.queues().await {
        Ok(q) if q.iter().any(|q| q.id == id) => {}
        Ok(_) => return error("there is no such queue"),
        Err(e) => return error(e),
    }
    if max_active.is_some_and(|n| !(1..=32).contains(&n)) {
        return error("simultaneous downloads must be between 1 and 32");
    }
    let schedule = windows.map(|w| (!w.is_empty()).then_some(Schedule { windows: w }));
    if let Some(Some(s)) = &schedule {
        if let Err(e) = s.check() {
            return error(e);
        }
    }
    if let Some(n) = max_active {
        if let Err(e) = manager.set_queue_limit(id, n).await {
            return error(e);
        }
    }
    if let Some(limit) = speed_limit {
        let value = (limit > 0).then_some(limit);
        if let Err(e) = manager.set_speed_limit(SpeedTarget::Queue(id), value).await {
            return error(e);
        }
    }
    if let Some(schedule) = schedule {
        if let Err(e) = manager.set_schedule(id, schedule).await {
            return error(e);
        }
    }
    queues(state).await
}

/// Follows what the manager says about the end-of-downloads action.
fn watch_action(state: &Arc<State>) {
    let mut events = state.manager.subscribe();
    let state = Arc::clone(state);
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            track(&state, &event);
            let mut a = state.action.lock().unwrap_or_else(|e| e.into_inner());
            match event {
                ManagerEvent::ActionCountdown { seconds_left, .. } => {
                    a.countdown = Some(seconds_left)
                }
                ManagerEvent::ActionCancelled { still_armed } => {
                    a.countdown = None;
                    if !still_armed {
                        a.armed = None;
                    }
                }
                ManagerEvent::ActionDone { action } => {
                    a.countdown = None;
                    a.armed = None;
                    a.message = Some(format!("Done: {}", action.describe()));
                }
                ManagerEvent::ActionFailed { message } => {
                    a.countdown = None;
                    a.armed = None;
                    a.message = Some(format!("It did not work: {message}"));
                }
                _ => {}
            }
        }
    });
}

/// The newest byte count of a download that is running, if there is one.
fn live_bytes(state: &State, id: i64, status: &mohona_store::Status) -> Option<u64> {
    if *status != mohona_store::Status::Downloading {
        return None;
    }
    let live = state.live.lock().unwrap_or_else(|e| e.into_inner());
    live.get(&id).map(|l| l.downloaded)
}

async fn details(state: &State, id: i64) -> Response {
    let rows = match state.manager.list().await {
        Ok(rows) => rows,
        Err(e) => return error(e),
    };
    let Some(row) = rows.into_iter().find(|r| r.id == id) else {
        return Response::Error {
            message: "there is no such download".into(),
        };
    };
    let live = state.live.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    let recent = live
        .get(&id)
        .filter(|l| now.duration_since(l.at).as_secs() < 5);
    let downloaded = recent.map_or(row.downloaded, |l| l.downloaded.max(row.downloaded));
    let speed = recent.map_or(0, |l| l.speed as u64);
    let connections = recent
        .map(|l| {
            l.parts
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let done = p.position >= p.end;
                    let active = !done && now.duration_since(p.moved_at).as_secs() < 3;
                    ConnectionInfo {
                        number: i as u32 + 1,
                        start: p.start,
                        end: p.end,
                        received: p.position.min(p.end).saturating_sub(p.start),
                        speed: if active { p.speed as u64 } else { 0 },
                        state: if done {
                            "done"
                        } else if active {
                            "active"
                        } else {
                            "waiting"
                        }
                        .into(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Response::Details {
        id,
        status: row.status.as_str().to_string(),
        downloaded,
        size: row.size,
        speed,
        connections,
    }
}

/// Keeps the newest progress and connection numbers of running downloads.
fn track(state: &State, event: &ManagerEvent) {
    use std::time::Instant;
    let mut live = state.live.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    match event {
        ManagerEvent::Progress { id, downloaded, .. } => {
            let l = live.entry(*id).or_insert_with(|| Live {
                downloaded: *downloaded,
                parts: Vec::new(),
                at: now,
                speed: 0.0,
                seen: (now, *downloaded),
            });
            l.downloaded = *downloaded;
            l.at = now;
            let seconds = now.duration_since(l.seen.0).as_secs_f64();
            if seconds >= 1.0 {
                let rate = downloaded.saturating_sub(l.seen.1) as f64 / seconds;
                l.speed = if l.speed == 0.0 {
                    rate
                } else {
                    l.speed * 0.5 + rate * 0.5
                };
                l.seen = (now, *downloaded);
            }
        }
        ManagerEvent::Parts { id, parts } => {
            let Some(l) = live.get_mut(id) else { return };
            let mut next = Vec::with_capacity(parts.len());
            for part in parts {
                // The same part as before is the one that began at the same byte.
                let old = l.parts.iter().find(|o| o.start == part.start);
                let (speed, moved_at) = match old {
                    Some(o) => {
                        let gained = part.position.saturating_sub(o.position);
                        let dt = now.duration_since(o.sampled).as_secs_f64().max(0.001);
                        let rate = gained as f64 / dt;
                        (
                            o.speed * 0.5 + rate * 0.5,
                            if gained > 0 { now } else { o.moved_at },
                        )
                    }
                    None => (0.0, now),
                };
                next.push(LivePart {
                    start: part.start,
                    end: part.end,
                    position: part.position,
                    speed,
                    moved_at,
                    sampled: now,
                });
            }
            l.parts = next;
        }
        ManagerEvent::State { id, status }
            if !matches!(
                status,
                mohona_store::Status::Downloading | mohona_store::Status::Probing
            ) =>
        {
            live.remove(id);
        }
        ManagerEvent::Removed { id } => {
            live.remove(id);
        }
        _ => {}
    }
}

/// The actions a client may arm: nothing that starts programs.
fn client_action(text: &str) -> Result<AfterAction, String> {
    match AfterAction::parse(text)? {
        a @ (AfterAction::Sleep | AfterAction::Hibernate | AfterAction::Shutdown) => Ok(a),
        _ => Err("use sleep, hibernate or shutdown".into()),
    }
}

async fn plan(state: &State) -> Response {
    let queues = match state.manager.queues().await {
        Ok(q) => q,
        Err(e) => return error(e),
    };
    let schedule = queues
        .first()
        .and_then(|q| q.schedule_json.as_deref())
        .and_then(|j| Schedule::from_json(j).ok());
    let windows = schedule
        .as_ref()
        .map(|s| s.windows.clone())
        .unwrap_or_default();
    let open_now = schedule
        .as_ref()
        .is_none_or(|s| s.open_window(Moment::now()).is_some());
    let a = state.action.lock().unwrap_or_else(|e| e.into_inner());
    Response::Plan {
        windows,
        open_now,
        action: a.armed.clone(),
        countdown: a.countdown,
        message: a.message.clone(),
    }
}

async fn set_plan(state: &State, windows: Vec<mohona_manager::Window>) -> Response {
    let queue = match state.manager.queues().await {
        Ok(q) => match q.first() {
            Some(q) => q.id,
            None => return error("there is no queue"),
        },
        Err(e) => return error(e),
    };
    let schedule = (!windows.is_empty()).then_some(Schedule { windows });
    if let Err(e) = state.manager.set_schedule(queue, schedule).await {
        return error(e);
    }
    plan(state).await
}

async fn set_after_action(state: &State, action: Option<String>, seconds: Option<u32>) -> Response {
    let (action, seconds) = match action {
        None => (None, 0),
        Some(text) => {
            let seconds = seconds.unwrap_or(60);
            if !(5..=3600).contains(&seconds) {
                return error("the countdown must be between 5 and 3600 seconds");
            }
            match client_action(&text) {
                Ok(a) => (Some(a), seconds),
                Err(e) => return error(e),
            }
        }
    };
    let name = action.as_ref().map(|a| a.name().to_string());
    if let Err(e) = state.manager.set_after_action(action, seconds).await {
        return error(e);
    }
    {
        let mut a = state.action.lock().unwrap_or_else(|e| e.into_inner());
        a.armed = name;
        a.countdown = None;
        a.message = None;
    }
    plan(state).await
}

/// Same answer in the same time whether the token is nearly right or not.
pub(crate) fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

async fn connection(stream: TcpStream, state: Arc<State>) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read).take(MAX_LINE as u64 + 1);
    loop {
        let mut line = String::new();
        reader.set_limit(MAX_LINE as u64 + 1);
        if reader.read_line(&mut line).await? == 0 || line.len() > MAX_LINE {
            return Ok(());
        }
        // Anything that is not a proper request (a web page speaking HTTP to
        // this port, for one) gets nothing and the connection is closed.
        let Ok(request) = serde_json::from_str::<Request>(&line) else {
            return Ok(());
        };
        if !same_token(&request.token, &state.token) {
            return Ok(());
        }
        let stop = request.call == Call::Shutdown;
        let response = handle(&state, request.call).await;
        let mut text = serde_json::to_string(&response).map_err(std::io::Error::other)?;
        text.push('\n');
        write.write_all(text.as_bytes()).await?;
        if stop {
            state.shutdown.notify_one();
            return Ok(());
        }
    }
}

fn error(e: impl std::fmt::Display) -> Response {
    Response::Error {
        message: e.to_string(),
    }
}

fn done(result: Result<(), ManagerError>) -> Response {
    match result {
        Ok(()) => Response::Done,
        Err(e) => error(e),
    }
}

async fn video_format(manager: &Manager) -> Response {
    let saved = |key: &'static str| async move {
        manager
            .setting(key)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    };
    let choice = mohona_media::ytdlp::Choice::from_settings(
        &saved(mohona_manager::VIDEO_QUALITY).await,
        &saved(mohona_manager::VIDEO_CONTAINER).await,
        &saved(mohona_manager::VIDEO_AUDIO).await,
    );
    Response::VideoFormat {
        quality: choice.quality,
        container: choice.container,
        audio: choice.audio,
    }
}

async fn sharing(manager: &Manager) -> Response {
    match manager.sharing().await {
        Ok(all) => Response::Sharing {
            items: all
                .into_iter()
                .map(|s| crate::protocol::SharedItem {
                    id: s.id as u64,
                    name: s.name,
                    total: s.total,
                    uploaded: s.uploaded,
                    seconds: s.seconds,
                })
                .collect(),
        },
        Err(e) => error(e),
    }
}

async fn torrent_speed(manager: &Manager) -> Response {
    let read = |key: &'static str| async move {
        manager
            .setting(key)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .filter(|v| *v > 0)
    };
    Response::TorrentSpeed {
        down_kb: read(mohona_manager::TORRENT_DOWN_KB).await,
        up_kb: read(mohona_manager::TORRENT_UP_KB).await,
    }
}

async fn seeding(state: &State) -> Response {
    let read = |key: &'static str| async move {
        state
            .manager
            .setting(key)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
    };
    Response::Seeding {
        ratio: read(mohona_manager::TORRENT_RATIO)
            .await
            .map(|r| (r * 100.0).round() as u32),
        minutes: read(mohona_manager::TORRENT_MINUTES)
            .await
            .map(|m| m as u64),
    }
}

async fn load_profiles(state: &State) -> Vec<crate::profiles::Profile> {
    let text = state
        .manager
        .setting(crate::profiles::SETTING)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

async fn load_rules(state: &State) -> Vec<crate::rules::Rule> {
    let text = state
        .manager
        .setting(crate::rules::SETTING)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

pub(crate) async fn handle(state: &State, call: Call) -> Response {
    let manager = &state.manager;
    match call {
        Call::Ping => Response::Pong {
            version: env!("CARGO_PKG_VERSION").to_string(),
            pid: std::process::id(),
        },
        Call::Add {
            url,
            dest_dir,
            cookies,
            referrer,
            user_agent,
            headers,
            sort,
            duplicates,
            queue,
            files,
            video,
            choice,
        } => {
            if let Some(folder) = crate::cloud::drive_folder(&url) {
                return add_drive_folder(state, folder, dest_dir, queue, duplicates).await;
            }
            let torrent_files = match files {
                None => None,
                Some(_) if !mohona_manager::is_torrent_address(&url) => {
                    return error("files can only be chosen for a torrent");
                }
                Some(list) => {
                    let mut list = list;
                    list.sort_unstable();
                    list.dedup();
                    if list.is_empty() || list.len() > 20_000 {
                        return error("choose at least one file of the torrent");
                    }
                    Some(list)
                }
            };
            let policy = match duplicates.as_deref() {
                None | Some("ask") => DuplicatePolicy::Ask,
                Some("skip") => DuplicatePolicy::Skip,
                Some("rename") => DuplicatePolicy::Rename,
                Some("overwrite") => DuplicatePolicy::Overwrite,
                Some(other) => {
                    return error(format!(
                        "\"{other}\" is not a duplicate policy (use ask, skip, rename or overwrite)"
                    ))
                }
            };
            // Rules fill in what the caller left open; they never override it.
            let rules = load_rules(state).await;
            let found = crate::rules::first_match(
                &rules,
                &crate::rules::Subject {
                    url: &url,
                    referrer: referrer.as_deref(),
                },
            );
            let (mut dest_dir, mut queue, mut rule_sort) = (dest_dir, queue, None);
            if let Some(rule) = found {
                if rule.then.block {
                    return Response::Error {
                        message: format!("blocked by the rule \"{}\"", rule.name),
                    };
                }
                if dest_dir.is_none() {
                    if let Some(folder) = &rule.then.folder {
                        dest_dir = Some(if crate::rules::is_full_path(folder) {
                            folder.clone()
                        } else {
                            folder_or_default(state, None)
                                .await
                                .join(folder.replace('\\', "/"))
                                .display()
                                .to_string()
                        });
                    }
                }
                if queue.is_none() {
                    queue = rule.then.queue;
                }
                rule_sort = rule.then.sort;
            }
            // A share link of a cloud service becomes the address of the file itself. Rules and
            // profiles above and below look at the address as it was pasted.
            let pasted = url.clone();
            let mut cookies = cookies;
            let url = match crate::cloud::resolve(&url) {
                Some(link) => {
                    // The file is on another host: the page's login cookies do not go there.
                    if crate::rules::host_of(&link.url) != crate::rules::host_of(&pasted) {
                        cookies = None;
                    }
                    link.url
                }
                None => url,
            };
            // A plugin for the site may give the real address (and what the site wants with it).
            let mut from_plugin = None;
            let url = if url == pasted {
                match plugin_for(state, &pasted).await {
                    Some((name, Err(why))) => {
                        return Response::Error {
                            message: format!(
                                "the plugin \"{name}\" could not handle this link: {why}"
                            ),
                        }
                    }
                    Some((_, Ok(found))) => {
                        if crate::rules::host_of(&found.url) != crate::rules::host_of(&pasted) {
                            cookies = None;
                        }
                        let address = found.url.clone();
                        from_plugin = Some(found);
                        address
                    }
                    None => url,
                }
            } else {
                url
            };
            let mut request = AddRequest::new(url, folder_or_default(state, dest_dir).await);
            request.sort_into_categories = match rule_sort {
                Some(on) if !sort => on,
                _ => sort || sorting_is_on(state).await,
            };
            request.duplicates = policy;
            request.queue_id = queue;
            // Only what a browser session legitimately carries: nothing that
            // weakens certificate checks or names proxies or key files.
            // The site's profile fills in what the caller left open.
            let profiles = load_profiles(state).await;
            let profile = crate::profiles::find(&profiles, &pasted).cloned();
            let mut headers = headers;
            let (mut referrer, mut user_agent) = (referrer, user_agent);
            if let Some(found) = from_plugin {
                referrer = referrer.or(found.referrer);
                user_agent = user_agent.or(found.user_agent);
                for (name, value) in found.headers {
                    if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                        headers.push((name, value));
                    }
                }
            }
            let mut connections = None;
            if let Some(p) = &profile {
                referrer = referrer.or_else(|| p.referrer.clone());
                user_agent = user_agent.or_else(|| p.user_agent.clone());
                for (name, value) in &p.headers {
                    if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
                        headers.push((name.clone(), value.clone()));
                    }
                }
                connections = p.connections;
            }
            let video_choice = choice;
            request.context = RequestContext {
                cookies,
                referrer,
                user_agent,
                headers,
                connections,
                torrent_files,
                video,
                video_choice,
                ..RequestContext::default()
            };
            match manager.add_checked(request).await {
                Ok(AddResult::Added(id)) => {
                    if let Some(limit) = profile.as_ref().and_then(|p| p.speed_limit) {
                        let _ = manager
                            .set_speed_limit(mohona_manager::SpeedTarget::Download(id), Some(limit))
                            .await;
                    }
                    Response::Added { id }
                }
                Ok(AddResult::Skipped(why)) => {
                    let (id, status, path, exists) = match &why {
                        mohona_manager::Duplicate::SameAddress { id, status, path } => {
                            let exists = path
                                .as_deref()
                                .is_some_and(|p| std::path::Path::new(p).exists());
                            (
                                Some(*id),
                                Some(status.as_str().to_string()),
                                path.clone(),
                                exists,
                            )
                        }
                        mohona_manager::Duplicate::FileExists { path } => {
                            (None, None, Some(path.display().to_string()), true)
                        }
                    };
                    Response::Skipped {
                        reason: why.to_string(),
                        id,
                        status,
                        path,
                        exists,
                    }
                }
                Err(e) => error(e),
            }
        }
        Call::GrabStart {
            url,
            depth,
            same_site,
            types,
            max_pages,
            robots,
        } => {
            let mut job = state.grab.lock().unwrap_or_else(|e| e.into_inner());
            if job.as_ref().is_some_and(|j| j.state == "running") {
                return error("the site grabber is already running: stop it first");
            }
            let options = mohona_grabber::Options {
                depth,
                same_site,
                types,
                max_pages: max_pages.map_or(100, |m| m as usize),
                robots,
                ..mohona_grabber::Options::default()
            };
            let client = match mohona_net::HttpClient::new(&mohona_net::ClientConfig::default()) {
                Ok(c) => c,
                Err(e) => return error(e),
            };
            let cancel = mohona_core::CancelToken::new();
            *job = Some(GrabJob {
                cancel: cancel.clone(),
                progress: mohona_grabber::Progress::default(),
                state: "running",
                result: None,
                error: None,
            });
            drop(job);
            let slot = state.grab.clone();
            tokio::spawn(async move {
                let progress_slot = slot.clone();
                let done = mohona_grabber::grab(&client, &url, &options, &cancel, |p| {
                    if let Some(j) = progress_slot
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_mut()
                    {
                        j.progress = p;
                    }
                })
                .await;
                let mut job = slot.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(j) = job
                    .as_mut()
                    .filter(|j| j.cancel.is_cancelled() || j.state == "running")
                {
                    match done {
                        Ok(r) => {
                            j.state = if r.note.as_deref() == Some("stopped") {
                                "stopped"
                            } else {
                                "done"
                            };
                            j.result = Some(r);
                        }
                        Err(e) => {
                            j.state = "failed";
                            j.error = Some(e);
                        }
                    }
                }
            });
            Response::Done
        }
        Call::GrabStop => {
            if let Some(j) = state
                .grab
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                j.cancel.cancel();
            }
            Response::Done
        }
        Call::GrabStatus => {
            let job = state.grab.lock().unwrap_or_else(|e| e.into_inner());
            match job.as_ref() {
                None => Response::Grab {
                    state: "idle".into(),
                    pages: 0,
                    found: 0,
                    current: String::new(),
                    files: Vec::new(),
                    skipped_by_robots: 0,
                    failed_pages: 0,
                    note: None,
                    error: None,
                },
                Some(j) => {
                    let r = j.result.as_ref();
                    Response::Grab {
                        state: j.state.into(),
                        pages: r.map_or(j.progress.pages, |r| r.pages) as u32,
                        found: r.map_or(j.progress.files, |r| r.files.len()) as u32,
                        current: j.progress.current.clone(),
                        files: r
                            .map(|r| {
                                r.files
                                    .iter()
                                    .map(|f| crate::protocol::GrabFile {
                                        url: f.url.clone(),
                                        name: f.name.clone(),
                                        group: f.group.into(),
                                        page: f.page.clone(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        skipped_by_robots: r.map_or(0, |r| r.skipped_by_robots as u32),
                        failed_pages: r.map_or(0, |r| r.failed_pages as u32),
                        note: r.and_then(|r| r.note.clone()),
                        error: j.error.clone(),
                    }
                }
            }
        }
        Call::TorrentFiles { url } => {
            if !mohona_manager::is_torrent_address(&url) {
                return error("that is not a magnet link or a .torrent address");
            }
            match manager.torrent_files(&url).await {
                Ok(listing) => Response::TorrentFiles {
                    name: listing.name,
                    files: listing
                        .files
                        .into_iter()
                        .map(|f| crate::protocol::TorrentFileInfo {
                            index: f.index,
                            path: f.path,
                            size: f.size,
                        })
                        .collect(),
                },
                Err(e) => error(e),
            }
        }
        Call::GetRules => Response::Rules {
            rules: load_rules(state).await,
        },
        Call::SetRules { rules } => {
            let old = load_rules(state).await;
            match crate::rules::check(rules, &old) {
                Err(message) => Response::Error { message },
                Ok(clean) => match serde_json::to_string(&clean) {
                    Ok(text) => done(manager.set_setting(crate::rules::SETTING, &text).await),
                    Err(e) => Response::Error {
                        message: e.to_string(),
                    },
                },
            }
        }
        Call::GetSeeding => seeding(state).await,
        Call::SetSeeding { ratio, minutes } => {
            if ratio.is_some_and(|r| r > 100_000) || minutes.is_some_and(|m| m > 10_000_000) {
                return Response::Error {
                    message:
                        "the ratio can be at most 1000, and the time at most 10 million minutes"
                            .into(),
                };
            }
            let text = |v: Option<String>| v.unwrap_or_default();
            for (key, value) in [
                (
                    mohona_manager::TORRENT_RATIO,
                    text(
                        ratio
                            .filter(|r| *r > 0)
                            .map(|r| (f64::from(r) / 100.0).to_string()),
                    ),
                ),
                (
                    mohona_manager::TORRENT_MINUTES,
                    text(minutes.filter(|m| *m > 0).map(|m| m.to_string())),
                ),
            ] {
                if let Err(e) = manager.set_setting(key, &value).await {
                    return error(e);
                }
            }
            seeding(state).await
        }
        Call::GetVideoFormat => video_format(manager).await,
        Call::SetVideoFormat {
            quality,
            container,
            audio,
        } => {
            use mohona_media::ytdlp::{AUDIO, CONTAINERS, QUALITIES};
            for (name, value, allowed) in [
                ("quality", &quality, QUALITIES),
                ("video format", &container, CONTAINERS),
                ("audio format", &audio, AUDIO),
            ] {
                if !allowed.contains(&value.as_str()) {
                    return Response::Error {
                        message: format!(
                            "\"{value}\" is not a {name} (one of: {})",
                            allowed.join(", ")
                        ),
                    };
                }
            }
            for (key, value) in [
                (mohona_manager::VIDEO_QUALITY, &quality),
                (mohona_manager::VIDEO_CONTAINER, &container),
                (mohona_manager::VIDEO_AUDIO, &audio),
            ] {
                if let Err(e) = manager.set_setting(key, value).await {
                    return error(e);
                }
            }
            video_format(manager).await
        }
        Call::GetSharing => sharing(manager).await,
        Call::StopSharing { id } => {
            match manager.stop_sharing(id as usize).await {
                Ok(true) => {}
                Ok(false) => {
                    return Response::Error {
                        message: "that torrent is not being shared".into(),
                    }
                }
                Err(e) => return error(e),
            }
            sharing(manager).await
        }
        Call::GetTorrentSpeed => torrent_speed(manager).await,
        Call::SetTorrentSpeed { down_kb, up_kb } => {
            // At most 4 million KB/s (about 4 GB/s) so that bytes per second fit in 32 bits.
            if down_kb.is_some_and(|k| k > 4_000_000) || up_kb.is_some_and(|k| k > 4_000_000) {
                return Response::Error {
                    message: "a speed limit can be at most 4000000 KB/s".into(),
                };
            }
            for (key, value) in [
                (mohona_manager::TORRENT_DOWN_KB, down_kb),
                (mohona_manager::TORRENT_UP_KB, up_kb),
            ] {
                let text = value
                    .filter(|k| *k > 0)
                    .map(|k| k.to_string())
                    .unwrap_or_default();
                if let Err(e) = manager.set_setting(key, &text).await {
                    return error(e);
                }
            }
            torrent_speed(manager).await
        }
        Call::GetUpdate => update_setting(manager, None).await,
        Call::SetUpdateRepo { repo } => {
            let repo = repo.trim().to_string();
            if !repo.is_empty() && !crate::update::valid_repo(&repo) {
                return Response::Error {
                    message: "the repository must look like owner/name".into(),
                };
            }
            match manager
                .set_setting(crate::update::REPO_SETTING, &repo)
                .await
            {
                Ok(()) => update_setting(manager, None).await,
                Err(e) => error(e),
            }
        }
        Call::CheckUpdate => {
            let repo = update_repo(manager).await;
            if repo.is_empty() {
                return Response::Error {
                    message: "no repository is set; give it as owner/name first".into(),
                };
            }
            let client = match mohona_net::HttpClient::new(&mohona_net::ClientConfig::default()) {
                Ok(c) => c,
                Err(e) => return error(e),
            };
            let api =
                std::env::var("MOHONA_UPDATE_API").unwrap_or_else(|_| crate::update::API.into());
            match crate::update::latest(&client, &api, &repo).await {
                Ok(release) => update_setting(manager, Some(release)).await,
                Err(message) => Response::Error { message },
            }
        }
        Call::GetScan => scan_setting(manager).await,
        Call::SetScan { mode, command } => {
            let mode = mode.trim().to_ascii_lowercase();
            if !matches!(mode.as_str(), "off" | "defender" | "custom") {
                return error("the scan mode is off, defender or custom");
            }
            if mode == "custom" && mohona_manager::scan::split_command(&command).is_empty() {
                return error("type the scanner program for a custom scan");
            }
            if let Err(e) = manager.set_setting(mohona_manager::scan::MODE, &mode).await {
                return error(e);
            }
            if let Err(e) = manager
                .set_setting(mohona_manager::scan::COMMAND, command.trim())
                .await
            {
                return error(e);
            }
            scan_setting(manager).await
        }
        Call::GetVideoBrowser => match manager.setting(mohona_manager::VIDEO_BROWSER).await {
            Ok(value) => Response::Setting {
                value: value.filter(|v| !v.is_empty()),
            },
            Err(e) => error(e),
        },
        Call::SetVideoBrowser { browser } => {
            let browser = browser.map(|b| b.trim().to_ascii_lowercase());
            if let Some(b) = browser.as_deref().filter(|b| !b.is_empty()) {
                if !mohona_media::ytdlp::BROWSERS.contains(&b) {
                    return Response::Error {
                        message: format!(
                            "\"{b}\" is not a browser yt-dlp knows ({})",
                            mohona_media::ytdlp::BROWSERS.join(", ")
                        ),
                    };
                }
            }
            done(
                manager
                    .set_setting(
                        mohona_manager::VIDEO_BROWSER,
                        browser.as_deref().unwrap_or(""),
                    )
                    .await,
            )
        }
        Call::GetTools => Response::Tools {
            tools: tool_list(state).await,
        },
        Call::SetTool { name, path } => {
            let Some(tool) = tool_named(&name) else {
                return Response::Error {
                    message: format!("\"{name}\" is not a tool Mohona knows (ffmpeg or yt-dlp)"),
                };
            };
            let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
            if let Some(p) = &path {
                if p.chars().any(char::is_control)
                    || tool.find(Some(std::path::Path::new(p))).is_none()
                {
                    return Response::Error {
                        message: format!(
                            "{p} is not {} (give the file, or the folder that holds it)",
                            tool.name
                        ),
                    };
                }
            }
            done(
                manager
                    .set_setting(&tool_key(&tool), path.as_deref().unwrap_or(""))
                    .await,
            )
        }
        Call::GetPlugins => Response::Plugins {
            folder: state.plugins_dir.display().to_string(),
            plugins: plugin_list(state).await,
        },
        Call::SetPlugin { name, enabled } => {
            if !plugin_list(state)
                .await
                .iter()
                .any(|p| p.name == name && p.problem.is_none())
            {
                return Response::Error {
                    message: format!("there is no plugin named \"{name}\""),
                };
            }
            let mut on = enabled_plugins(state).await;
            on.retain(|n| *n != name);
            if enabled {
                on.push(name);
            }
            match serde_json::to_string(&on) {
                Ok(text) => done(
                    manager
                        .set_setting(crate::plugins::ENABLED_SETTING, &text)
                        .await,
                ),
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Call::SyncNow { folder } => {
            let folder = match folder {
                Some(f) => Some(f),
                None => sync_folder(state).await,
            };
            match folder {
                None => Response::Error {
                    message: "no folder: give one, or choose it with `mohona sync set FOLDER`"
                        .into(),
                },
                Some(f) => match run_sync(state, std::path::Path::new(&f)).await {
                    Ok((applied, wrote)) => Response::Synced { applied, wrote },
                    Err(message) => Response::Error { message },
                },
            }
        }
        Call::SetSync { folder } => match folder {
            Some(f) if !std::path::Path::new(&f).is_dir() => Response::Error {
                message: format!("{f} is not a folder"),
            },
            Some(f) => done(manager.set_setting(crate::sync::FOLDER_SETTING, &f).await),
            None => done(manager.set_setting(crate::sync::FOLDER_SETTING, "").await),
        },
        Call::GetSync => Response::SyncFolder {
            folder: sync_folder(state).await,
        },
        Call::GetRemote => Response::Remote {
            port: remote_setting(state).await,
            listening: state.remote_port.load(std::sync::atomic::Ordering::Relaxed),
        },
        Call::SetRemote { port } => done(manager.set_setting(REMOTE_KEY, &port.to_string()).await),
        Call::MakeToken { name, scope } => {
            match crate::api::make_token(state, name.clone(), scope.clone()).await {
                Ok((id, token)) => Response::Token {
                    id,
                    name,
                    scope,
                    token,
                },
                Err(message) => Response::Error { message },
            }
        }
        Call::GetProfiles => Response::Profiles {
            profiles: load_profiles(state).await,
        },
        Call::SetProfiles { profiles } => {
            let old = load_profiles(state).await;
            match crate::profiles::check(profiles, &old) {
                Err(message) => Response::Error { message },
                Ok(clean) => match serde_json::to_string(&clean) {
                    Ok(text) => done(manager.set_setting(crate::profiles::SETTING, &text).await),
                    Err(e) => Response::Error {
                        message: e.to_string(),
                    },
                },
            }
        }
        Call::TestRules { url, referrer } => {
            let rules = load_rules(state).await;
            let rule = crate::rules::first_match(
                &rules,
                &crate::rules::Subject {
                    url: &url,
                    referrer: referrer.as_deref(),
                },
            )
            .cloned();
            Response::RuleMatch { rule }
        }
        Call::Stream {
            url,
            dest_dir,
            cookies,
            referrer,
            user_agent,
            headers,
            name,
        } => {
            let folder = folder_or_default(state, dest_dir).await;
            let shown = folder.display().to_string();
            let env: Vec<(String, String)> = tool_list(state)
                .await
                .into_iter()
                .filter(|t| t.name == "ffmpeg")
                .filter_map(|t| t.found.map(|f| ("MOHONA_FFMPEG".to_string(), f)))
                .collect();
            match state.streams.start(
                env,
                StreamJob {
                    url,
                    folder,
                    cookies,
                    referrer,
                    user_agent,
                    headers,
                    name,
                },
            ) {
                Ok(job) => Response::Started { job, folder: shown },
                Err(message) => Response::Error { message },
            }
        }
        Call::ReceiveStart {
            name,
            mime,
            dest_dir,
        } => {
            let folder = folder_or_default(state, dest_dir).await;
            match state.receiver.start(&folder, &name, mime) {
                Ok(transfer) => Response::Receiving { transfer },
                Err(message) => Response::Error { message },
            }
        }
        Call::ReceivePiece {
            transfer,
            index,
            data,
        } => match state.receiver.piece(transfer, index, &data) {
            Ok(()) => Response::Done,
            Err(message) => Response::Error { message },
        },
        Call::ReceiveEnd { transfer } => match state.receiver.finish(transfer) {
            Ok(path) => Response::Saved {
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path: path.display().to_string(),
            },
            Err(message) => Response::Error { message },
        },
        Call::ReceiveAbort { transfer } => {
            state.receiver.abort(transfer);
            Response::Done
        }
        Call::Jobs => Response::Jobs {
            jobs: state.streams.list(),
        },
        Call::SettingGet { key } => match setting_key(&key) {
            Err(message) => Response::Error { message },
            Ok(()) => match manager.setting(&key).await {
                Ok(value) => Response::Setting { value },
                Err(e) => error(e),
            },
        },
        Call::SettingSet { key, value } => match setting_key(&key) {
            Err(message) => Response::Error { message },
            Ok(()) if value.len() > MAX_SETTING => Response::Error {
                message: "the setting is too large".into(),
            },
            Ok(()) => done(manager.set_setting(&key, &value).await),
        },
        Call::GetDetails { id } => details(state, id).await,
        Call::List => match manager.list().await {
            Ok(rows) => {
                let mut downloads = Vec::with_capacity(rows.len());
                for r in rows {
                    let expired = r.status == mohona_store::Status::Failed
                        && r.error.as_deref().is_some_and(|e| {
                            e.contains("answered 403") || e.contains("answered 410")
                        });
                    let page = if expired {
                        manager
                            .request_context(r.id)
                            .await
                            .ok()
                            .and_then(|c| c.referrer)
                    } else {
                        None
                    };
                    downloads.push(DownloadInfo {
                        id: r.id,
                        url: r.url,
                        filename: r.filename,
                        status: r.status.as_str().to_string(),
                        downloaded: live_bytes(state, r.id, &r.status)
                            .map_or(r.downloaded, |l| l.max(r.downloaded)),
                        size: r.size,
                        error: r.error,
                        path: r.path,
                        page,
                        expired,
                    });
                }
                Response::Downloads { downloads }
            }
            Err(e) => error(e),
        },
        Call::Refresh {
            id,
            url,
            cookies,
            referrer,
            user_agent,
        } => done(
            manager
                .refresh_link(
                    id,
                    &url,
                    RequestContext {
                        cookies,
                        referrer,
                        user_agent,
                        ..RequestContext::default()
                    },
                )
                .await,
        ),
        Call::GetCategories => categories(state).await,
        Call::AddCategory {
            name,
            extensions,
            folder,
        } => add_category(state, name, extensions, folder).await,
        Call::RemoveCategory { id } => match manager.remove_category(id).await {
            Ok(()) => categories(state).await,
            Err(e) => error(e),
        },
        Call::GetQueues => queues(state).await,
        Call::AddQueue { name } => match check_queue_name(&name) {
            Err(message) => Response::Error { message },
            Ok(name) => match manager.add_queue(name).await {
                Ok(_) => queues(state).await,
                Err(e) => error(e),
            },
        },
        Call::RenameQueue { id, name } => match check_queue_name(&name) {
            Err(message) => Response::Error { message },
            Ok(name) => match manager.rename_queue(id, name).await {
                Ok(()) => queues(state).await,
                Err(e) => error(e),
            },
        },
        Call::RemoveQueue { id } => match manager.remove_queue(id).await {
            Ok(()) => queues(state).await,
            Err(e) => error(e),
        },
        Call::SetQueue {
            id,
            max_active,
            speed_limit,
            windows,
        } => set_queue(state, id, max_active, speed_limit, windows).await,
        Call::GetPlan => plan(state).await,
        Call::SetPlan { windows } => set_plan(state, windows).await,
        Call::SetAfterAction {
            action,
            countdown_seconds,
        } => set_after_action(state, action, countdown_seconds).await,
        Call::CancelAction => set_after_action(state, None, None).await,
        Call::GetConfig => config(state).await,
        Call::SetConfig {
            download_folder,
            speed_limit,
            max_active,
            notifications,
            keep_in_tray,
            sort_by_type,
        } => {
            let change = ConfigChange {
                sort_by_type,
                download_folder,
                speed_limit,
                max_active,
                notifications,
                keep_in_tray,
            };
            set_config(state, change).await
        }
        Call::Pause { id } => done(manager.pause(id).await),
        Call::Resume { id } => done(manager.resume(id).await),
        Call::Cancel { id } => done(manager.cancel(id).await),
        Call::Remove { id, delete_file } => done(manager.remove(id, delete_file).await),
        Call::Shutdown => Response::Done,
    }
}

async fn sorting_is_on(state: &State) -> bool {
    matches!(state.manager.setting(SORT_KEY).await, Ok(Some(v)) if v == "1")
}

async fn categories(state: &State) -> Response {
    match state.manager.categories().await {
        Ok(rows) => Response::Categories {
            categories: rows
                .into_iter()
                .map(|c| CategoryInfo {
                    id: c.id,
                    name: c.name,
                    extensions: c.extensions,
                    folder: c.folder,
                })
                .collect(),
        },
        Err(e) => error(e),
    }
}

/// A category is checked before it is saved: a plain name, file types that
/// no other category already has, and a folder that cannot climb out of the
/// download folder.
async fn add_category(
    state: &State,
    name: String,
    extensions: Vec<String>,
    folder: Option<String>,
) -> Response {
    let name = name.trim().to_string();
    if name.is_empty()
        || name.chars().count() > 40
        || name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
    {
        return error("the name must be 1 to 40 characters and not contain \\ / : * ? \" < > |");
    }
    let extensions: Vec<String> = extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    if extensions.is_empty() {
        return error("give at least one file type, such as zip");
    }
    if let Some(bad) = extensions
        .iter()
        .find(|e| e.len() > 12 || !e.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return error(format!(
            "\"{bad}\" is not a file type (letters and digits only)"
        ));
    }
    let folder = folder
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty());
    if let Some(f) = &folder {
        let path = std::path::Path::new(f);
        let climbs = path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
        if f.chars().any(char::is_control) || climbs {
            return error("the folder must be a full path, or a folder name without \"..\"");
        }
    }
    let existing = match state.manager.categories().await {
        Ok(c) => c,
        Err(e) => return error(e),
    };
    if existing.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
        return error(format!("there is already a category named {name}"));
    }
    for e in &extensions {
        if let Some(other) = existing.iter().find(|c| c.extensions.contains(e)) {
            return error(format!("\"{e}\" already belongs to {}", other.name));
        }
    }
    match state.manager.add_category(name, extensions, folder).await {
        Ok(_) => categories(state).await,
        Err(e) => error(e),
    }
}

/// The settings key of the folder downloads go to.
const FOLDER_KEY: &str = "download_folder";
/// The settings key of the global speed limit (the manager's own).
const SPEED_KEY: &str = "speed_limit";
const NOTIFY_KEY: &str = "notifications";
const TRAY_KEY: &str = "keep_in_tray";
const SORT_KEY: &str = "sort_by_type";

/// The folder a client named, else the folder the person chose, else the default.
async fn folder_or_default(state: &State, named: Option<String>) -> PathBuf {
    if let Some(named) = named {
        return PathBuf::from(named);
    }
    match state.manager.setting(FOLDER_KEY).await {
        Ok(Some(chosen)) if !chosen.is_empty() => PathBuf::from(chosen),
        _ => state.default_dir.clone(),
    }
}

/// A shared Google Drive folder: lists it through Drive's embedded folder view and adds each file
/// into a folder of the same name (and the folders inside it, a few levels down).
async fn add_drive_folder(
    state: &State,
    id: String,
    dest_dir: Option<String>,
    queue: Option<i64>,
    duplicates: Option<String>,
) -> Response {
    const MAX_FILES: usize = 500;
    const MAX_DEPTH: usize = 4;
    let client = match mohona_net::HttpClient::new(&mohona_net::ClientConfig::default()) {
        Ok(c) => c,
        Err(e) => return error(e),
    };
    let base = folder_or_default(state, dest_dir).await;
    let mut work: Vec<(String, Option<PathBuf>, usize)> = vec![(id, None, 0)];
    let (mut first, mut added, mut skipped, mut seen) = (None, 0usize, 0usize, 0usize);
    let mut last_problem = None;
    while let Some((folder, dir, depth)) = work.pop() {
        let page = format!("https://drive.google.com/embeddedfolderview?id={folder}");
        let mut body = match client.get(&page).await {
            Ok(b) => b,
            Err(e) => {
                last_problem = Some(format!("could not read the folder: {e}"));
                continue;
            }
        };
        let mut data = Vec::new();
        while let Ok(Some(chunk)) = body.chunk().await {
            data.extend_from_slice(&chunk);
            if data.len() > 8 << 20 {
                break;
            }
        }
        let html = String::from_utf8_lossy(&data).into_owned();
        let dir = dir.unwrap_or_else(|| {
            base.join(crate::cloud::safe_name(
                &crate::cloud::parse_folder_title(&html).unwrap_or_default(),
            ))
        });
        for entry in crate::cloud::parse_folder_listing(&html) {
            if entry.is_folder {
                if depth < MAX_DEPTH {
                    let sub = dir.join(crate::cloud::safe_name(&entry.name));
                    work.push((entry.id, Some(sub), depth + 1));
                }
                continue;
            }
            seen += 1;
            if seen > MAX_FILES {
                break;
            }
            let answer = Box::pin(handle(
                state,
                Call::Add {
                    url: format!("https://drive.google.com/file/d/{}/view", entry.id),
                    dest_dir: Some(dir.display().to_string()),
                    cookies: None,
                    referrer: None,
                    user_agent: None,
                    headers: vec![],
                    sort: false,
                    duplicates: Some(duplicates.clone().unwrap_or_else(|| "skip".into())),
                    queue,
                    files: None,
                    video: false,
                    choice: None,
                },
            ))
            .await;
            match answer {
                Response::Added { id } => {
                    added += 1;
                    first.get_or_insert(id);
                }
                Response::Skipped { .. } => skipped += 1,
                Response::Error { message } => last_problem = Some(message),
                _ => {}
            }
        }
    }
    match (first, skipped) {
        (Some(id), _) => Response::Added { id },
        (None, n) if n > 0 => Response::Skipped {
            reason: format!("all {n} files of the folder are already in the list"),
            id: None,
            status: None,
            path: None,
            exists: false,
        },
        _ => {
            let _ = added;
            Response::Error {
                message: last_problem.unwrap_or_else(|| {
                    "no files could be listed in this Google Drive folder (is it shared with anyone who has the link?)".into()
                }),
            }
        }
    }
}

/// The yes/no settings are stored as "1" and "0"; both are on until switched off.
async fn flag(state: &State, key: &str) -> bool {
    !matches!(state.manager.setting(key).await, Ok(Some(v)) if v == "0")
}

async fn config(state: &State) -> Response {
    let manager = &state.manager;
    let folder = folder_or_default(state, None).await;
    let speed_limit = manager
        .setting(SPEED_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0);
    let max_active = match manager.queues().await {
        Ok(queues) => queues.first().map_or(4, |q| q.max_active),
        Err(e) => return error(e),
    };
    Response::Config {
        download_folder: folder.display().to_string(),
        default_folder: state.default_dir.display().to_string(),
        speed_limit,
        max_active,
        notifications: flag(state, NOTIFY_KEY).await,
        keep_in_tray: flag(state, TRAY_KEY).await,
        sort_by_type: matches!(state.manager.setting(SORT_KEY).await, Ok(Some(v)) if v == "1"),
    }
}

struct ConfigChange {
    download_folder: Option<String>,
    speed_limit: Option<u64>,
    max_active: Option<u32>,
    notifications: Option<bool>,
    keep_in_tray: Option<bool>,
    sort_by_type: Option<bool>,
}

async fn set_config(state: &State, change: ConfigChange) -> Response {
    let ConfigChange {
        download_folder: folder,
        speed_limit,
        max_active,
        notifications,
        keep_in_tray,
        sort_by_type,
    } = change;
    let manager = &state.manager;
    // Check everything first, so a refused value changes nothing.
    let folder = match folder.map(|f| f.trim().to_string()) {
        None => None,
        Some(f) if f.is_empty() => Some(String::new()),
        Some(f) => {
            let path = PathBuf::from(&f);
            if !path.is_absolute() || f.chars().any(char::is_control) {
                return error(
                    "the download folder must be a full path such as C:\\Users\\you\\Downloads",
                );
            }
            if let Err(e) = std::fs::create_dir_all(&path) {
                return error(format!("cannot use the folder {f}: {e}"));
            }
            Some(f)
        }
    };
    if max_active.is_some_and(|n| !(1..=32).contains(&n)) {
        return error("simultaneous downloads must be between 1 and 32");
    }
    if let Some(f) = folder {
        let result = if f.is_empty() {
            manager.set_setting(FOLDER_KEY, "").await
        } else {
            manager.set_setting(FOLDER_KEY, &f).await
        };
        if let Err(e) = result {
            return error(e);
        }
    }
    if let Some(limit) = speed_limit {
        let value = (limit > 0).then_some(limit);
        if let Err(e) = manager.set_speed_limit(SpeedTarget::Global, value).await {
            return error(e);
        }
    }
    for (key, value) in [
        (NOTIFY_KEY, notifications),
        (TRAY_KEY, keep_in_tray),
        (SORT_KEY, sort_by_type),
    ] {
        if let Some(on) = value {
            if let Err(e) = manager.set_setting(key, if on { "1" } else { "0" }).await {
                return error(e);
            }
        }
    }
    if let Some(n) = max_active {
        match manager.queues().await {
            Ok(queues) => {
                for q in queues.iter().take(1) {
                    if let Err(e) = manager.set_queue_limit(q.id, n).await {
                        return error(e);
                    }
                }
            }
            Err(e) => return error(e),
        }
    }
    config(state).await
}

/// The largest setting value the daemon keeps for a client.
const MAX_SETTING: usize = 64 * 1024;

/// Clients may only touch `extension.*` settings (never the engine's own).
fn setting_key(key: &str) -> Result<(), String> {
    let ok = key.len() <= 64
        && key.starts_with("extension.")
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(format!("\"{key}\" is not a setting clients may use"))
    }
}
