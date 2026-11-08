//! What the Mohona app does with the engine, as plain functions.
//!
//! The window code (Tauri) only forwards its commands here, so everything
//! that can go wrong is tested without a window. The app is a client of the
//! daemon like the command line and the browser extension: it finds the
//! daemon through the info file, starts it when needed, and asks for the list
//! of downloads about once a second.

mod rows;
mod settings;
mod speed;
pub mod startup;
mod watch;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mohona_daemon::{Call, Client, ClientError, Response};
use serde::Serialize;

pub use rows::{name_of, Row};
pub use settings::{merge_extension, ExtensionSettings};
pub use speed::SpeedTracker;
pub use watch::{Notice, NoticeAction, Watcher};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Mohona is not running")]
    NotRunning,
    #[error("{0}")]
    Engine(String),
    #[error("{0}")]
    Input(String),
}

impl From<ClientError> for AppError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::NotRunning => AppError::NotRunning,
            ClientError::Protocol(m) => AppError::Engine(m),
        }
    }
}

/// Errors reach the window as plain text.
impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EngineStatus {
    pub running: bool,
    pub version: Option<String>,
    /// Where finished downloads go by default is the engine's business; the
    /// app only shows this when it knows it.
    pub message: Option<String>,
}

/// The engine's own settings, as the settings page shows them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub download_folder: String,
    pub default_folder: String,
    /// Bytes per second; `None` for no limit.
    pub speed_limit: Option<u64>,
    pub max_active: u32,
    /// Tell the person when a download finishes or fails.
    pub notifications: bool,
    /// Closing the window leaves the app in the tray.
    pub keep_in_tray: bool,
    /// Finished downloads move into the folder of their category.
    pub sort_by_type: bool,
}

/// What may be changed; what is `None` stays as it is.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConfigChange {
    pub download_folder: Option<String>,
    /// Bytes per second; 0 removes the limit.
    pub speed_limit: Option<u64>,
    pub max_active: Option<u32>,
    pub notifications: Option<bool>,
    pub keep_in_tray: Option<bool>,
    pub sort_by_type: Option<bool>,
}

/// A group of file types that is sorted into its own folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub extensions: Vec<String>,
    /// As set; none means a folder with the category's name.
    pub folder: Option<String>,
}

/// One time window of the schedule, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanWindow {
    /// `mon` to `sun`; empty means every day.
    pub days: Vec<String>,
    /// `HH:MM`
    pub start: String,
    /// `HH:MM`; earlier than `start` means after midnight.
    pub end: String,
    /// Bytes per second while this window is open; none: the usual limit.
    pub speed_limit: Option<u64>,
}

/// One download queue, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub id: i64,
    pub name: String,
    pub max_active: u32,
    pub speed_limit: Option<u64>,
    pub windows: Vec<PlanWindow>,
    pub open_now: bool,
}

/// What the site grabber is to look for.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabOptions {
    pub url: String,
    pub depth: u32,
    pub same_site: bool,
    pub types: Vec<String>,
    pub max_pages: Option<u32>,
    pub robots: bool,
}

/// A file the site grabber found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabFile {
    pub url: String,
    pub name: String,
    pub group: String,
    /// The page that links to it.
    pub page: String,
}

/// How the site grabber is doing, and what it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabState {
    /// `idle`, `running`, `done`, `stopped` or `failed`.
    pub state: String,
    pub pages: u32,
    pub found: u32,
    pub current: String,
    pub files: Vec<GrabFile>,
    pub skipped_by_robots: u32,
    pub failed_pages: u32,
    pub note: Option<String>,
    pub error: Option<String>,
}

/// What came of adding the files the grabber found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrabAdded {
    pub added: u32,
    /// Files that were not added, with the reason (already in the list, for one).
    pub skipped: Vec<String>,
}

/// One file inside a torrent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    pub index: u32,
    pub path: String,
    pub size: u64,
}

/// What a torrent holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentList {
    pub name: String,
    pub files: Vec<TorrentFile>,
}

/// One rule, as the page shows it: what to look for, and what to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// 0 for a rule that is not saved yet.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub url_contains: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub queue: Option<i64>,
    #[serde(default)]
    pub sort: Option<bool>,
    #[serde(default)]
    pub block: bool,
}

impl From<mohona_daemon::rules::Rule> for Rule {
    fn from(r: mohona_daemon::rules::Rule) -> Self {
        Rule {
            id: r.id,
            name: r.name,
            enabled: r.enabled,
            host: r.when.host,
            extensions: r.when.extensions,
            url_contains: r.when.url_contains,
            folder: r.then.folder,
            queue: r.then.queue,
            sort: r.then.sort,
            block: r.then.block,
        }
    }
}

/// The speed limits of torrents, in KB/s (none: no limit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentSpeed {
    pub down_kb: Option<u32>,
    pub up_kb: Option<u32>,
}

/// How finished torrents are shared, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seeding {
    /// Stop at this upload ratio, in hundredths (150 is 1.5).
    pub ratio: Option<u32>,
    /// Stop after this many minutes.
    pub minutes: Option<u64>,
}

/// One outside program (ffmpeg, yt-dlp), as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    /// What Mohona needs it for.
    pub purpose: String,
    /// The path the person chose, if any.
    pub chosen: Option<String>,
    /// The file that will be used, if one is found.
    pub found: Option<String>,
    pub page: String,
    pub windows_page: String,
    pub winget: String,
}

/// Whether the app may open this address for the person: only the download pages of the tools.
pub fn is_tool_page(url: &str) -> bool {
    mohona_daemon::TOOL_PAGES.contains(&url)
}

/// One site profile, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteProfile {
    /// 0 for a profile that is not saved yet.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub host: String,
    #[serde(default)]
    pub connections: Option<u32>,
    /// Bytes per second for each download from the site.
    #[serde(default)]
    pub speed_limit: Option<u64>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub referrer: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

impl From<mohona_daemon::profiles::Profile> for SiteProfile {
    fn from(p: mohona_daemon::profiles::Profile) -> Self {
        SiteProfile {
            id: p.id,
            name: p.name,
            enabled: p.enabled,
            host: p.host,
            connections: p.connections,
            speed_limit: p.speed_limit,
            user_agent: p.user_agent,
            referrer: p.referrer,
            headers: p.headers,
        }
    }
}

impl From<SiteProfile> for mohona_daemon::profiles::Profile {
    fn from(p: SiteProfile) -> Self {
        mohona_daemon::profiles::Profile {
            id: p.id,
            name: p.name,
            enabled: p.enabled,
            host: p.host,
            connections: p.connections,
            speed_limit: p.speed_limit,
            user_agent: p.user_agent,
            referrer: p.referrer,
            headers: p.headers,
        }
    }
}

/// One connection of a running download, as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub number: u32,
    pub start: u64,
    pub end: u64,
    pub received: u64,
    /// Bytes per second.
    pub speed: u64,
    /// `active`, `waiting` or `done`.
    pub state: String,
}

/// What a download is doing now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    pub id: i64,
    pub status: String,
    pub downloaded: u64,
    pub size: Option<u64>,
    /// Bytes per second, all connections together.
    pub speed: u64,
    /// Empty for a download on a single connection.
    pub connections: Vec<Connection>,
}

/// What may be changed in a queue; what is `None` stays.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueueChange {
    pub max_active: Option<u32>,
    /// Bytes per second; 0 removes the limit.
    pub speed_limit: Option<u64>,
    /// An empty list means any time.
    pub windows: Option<Vec<PlanWindow>>,
}

/// The schedule and the armed end-of-downloads action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub windows: Vec<PlanWindow>,
    pub open_now: bool,
    /// `sleep`, `hibernate` or `shutdown`.
    pub action: Option<String>,
    /// Seconds left while the countdown runs.
    pub countdown: Option<u32>,
    pub message: Option<String>,
}

impl From<mohona_manager::Window> for PlanWindow {
    fn from(w: mohona_manager::Window) -> Self {
        PlanWindow {
            days: w.days,
            start: w.start,
            end: w.end,
            speed_limit: w.speed_limit,
        }
    }
}

impl From<PlanWindow> for mohona_manager::Window {
    fn from(w: PlanWindow) -> Self {
        mohona_manager::Window {
            days: w.days,
            start: w.start,
            end: w.end,
            speed_limit: w.speed_limit,
        }
    }
}

/// The settings key the browser extension and the app share.
const EXTENSION_KEY: &str = "extension.settings";

/// How to reach (and start) the engine.
#[derive(Debug, Clone)]
pub struct Engine {
    pub info_path: PathBuf,
    /// The `mohona` program that runs `daemon`; `None` when it was not found.
    pub program: Option<PathBuf>,
    /// The database the started engine should use (`None`: its default).
    pub database: Option<PathBuf>,
}

/// How long to wait for a started engine to answer.
const START_WAIT: Duration = Duration::from_secs(10);

impl Engine {
    /// The default places: the info file, and the `mohona` program named by
    /// `MOHONA_PROGRAM`, or next to the app, or on the `PATH`.
    pub fn standard() -> Self {
        Engine {
            info_path: mohona_daemon::default_info_path(),
            program: find_program(
                std::env::var_os("MOHONA_PROGRAM"),
                std::env::current_exe().ok().as_deref(),
                std::env::var_os("PATH"),
            ),
            database: None,
        }
    }

    fn client(&self) -> Result<Client, AppError> {
        Client::from_info_file(&self.info_path).map_err(AppError::from)
    }

    async fn call(&self, call: Call) -> Result<Response, AppError> {
        match self.client()?.call(call).await? {
            Response::Error { message } => Err(AppError::Engine(message)),
            other => Ok(other),
        }
    }

    pub async fn status(&self) -> EngineStatus {
        match self.call(Call::Ping).await {
            Ok(Response::Pong { version, .. }) => EngineStatus {
                running: true,
                version: Some(version),
                message: None,
            },
            Err(AppError::NotRunning) => EngineStatus {
                running: false,
                version: None,
                message: None,
            },
            Err(e) => EngineStatus {
                running: false,
                version: None,
                message: Some(e.to_string()),
            },
            Ok(_) => EngineStatus {
                running: false,
                version: None,
                message: Some("unexpected answer from the engine".into()),
            },
        }
    }

    /// Starts the engine when it is not running; waits until it answers.
    pub async fn start(&self) -> Result<EngineStatus, AppError> {
        let status = self.status().await;
        if status.running {
            return Ok(status);
        }
        let program = self.program.as_ref().ok_or_else(|| {
            AppError::Engine("the mohona program was not found next to the app".into())
        })?;
        spawn(program, &self.info_path, self.database.as_deref())
            .map_err(|e| AppError::Engine(format!("could not start Mohona: {e}")))?;
        let started = Instant::now();
        while started.elapsed() < START_WAIT {
            let status = self.status().await;
            if status.running {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(AppError::Engine(
            "Mohona was started but did not answer in time".into(),
        ))
    }

    /// Every download, with speed and time left worked out by `tracker`.
    pub async fn list(&self, tracker: &mut SpeedTracker) -> Result<Vec<Row>, AppError> {
        match self.call(Call::List).await? {
            Response::Downloads { downloads } => Ok(tracker.rows(downloads, Instant::now())),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// Adds one address; returns the new download's id.
    pub async fn add(&self, url: &str, folder: Option<&str>) -> Result<i64, AppError> {
        self.add_to(url, folder, None).await
    }

    /// Like [`Engine::add`], into a chosen queue (`None`: the main queue).
    pub async fn add_to(
        &self,
        url: &str,
        folder: Option<&str>,
        queue: Option<i64>,
    ) -> Result<i64, AppError> {
        self.add_with(url, folder, queue, None).await
    }

    /// What is inside a torrent (a magnet link can take up to 90 seconds while peers are found).
    pub async fn torrent_files(&self, url: &str) -> Result<TorrentList, AppError> {
        let url = url.trim();
        match self
            .call(Call::TorrentFiles {
                url: url.to_string(),
            })
            .await?
        {
            Response::TorrentFiles { name, files } => Ok(TorrentList {
                name,
                files: files
                    .into_iter()
                    .map(|f| TorrentFile {
                        index: f.index,
                        path: f.path,
                        size: f.size,
                    })
                    .collect(),
            }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// Starts the site grabber (one run at a time).
    pub async fn grab_start(&self, options: GrabOptions) -> Result<(), AppError> {
        let url = options.url.trim().to_string();
        if url.is_empty() {
            return Err(AppError::Input("type or paste a web address".into()));
        }
        if options.depth > 5 {
            return Err(AppError::Input("the depth can be at most 5".into()));
        }
        self.call(Call::GrabStart {
            url,
            depth: options.depth,
            same_site: options.same_site,
            types: options.types,
            max_pages: options.max_pages,
            robots: options.robots,
        })
        .await?;
        Ok(())
    }

    /// How the site grabber is doing, and (once it is over) what it found.
    pub async fn grab_status(&self) -> Result<GrabState, AppError> {
        match self.call(Call::GrabStatus).await? {
            Response::Grab {
                state,
                pages,
                found,
                current,
                files,
                skipped_by_robots,
                failed_pages,
                note,
                error,
            } => Ok(GrabState {
                state,
                pages,
                found,
                current,
                files: files
                    .into_iter()
                    .map(|f| GrabFile {
                        url: f.url,
                        name: f.name,
                        group: f.group,
                        page: f.page,
                    })
                    .collect(),
                skipped_by_robots,
                failed_pages,
                note,
                error,
            }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// Stops the site grabber; what it found so far stays.
    pub async fn grab_stop(&self) -> Result<(), AppError> {
        self.call(Call::GrabStop).await?;
        Ok(())
    }

    /// Adds the files the grabber found (the page that links to each is sent as its referrer,
    /// which many sites want), into a folder and queue. Files already in the list are skipped.
    pub async fn grab_download(
        &self,
        files: Vec<GrabFile>,
        folder: Option<&str>,
        queue: Option<i64>,
    ) -> Result<GrabAdded, AppError> {
        if files.is_empty() {
            return Err(AppError::Input("choose at least one file".into()));
        }
        let folder = folder.map(str::trim).filter(|f| !f.is_empty());
        let mut added = 0;
        let mut skipped = Vec::new();
        for file in files {
            let lower = file.url.to_ascii_lowercase();
            if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                skipped.push(format!("{}: not a web address", file.name));
                continue;
            }
            match self
                .call(Call::Add {
                    url: file.url.clone(),
                    dest_dir: folder.map(str::to_string),
                    cookies: None,
                    referrer: Some(file.page.clone()).filter(|p| p.starts_with("http")),
                    user_agent: None,
                    headers: Vec::new(),
                    sort: false,
                    duplicates: Some("skip".into()),
                    queue,
                    files: None,
                    video: false,
                })
                .await
            {
                Ok(Response::Added { .. }) => added += 1,
                Ok(Response::Skipped { reason }) => {
                    skipped.push(format!("{}: {reason}", file.name))
                }
                Ok(_) => {}
                Err(e) => skipped.push(format!("{}: {e}", file.name)),
            }
        }
        Ok(GrabAdded { added, skipped })
    }

    /// Add a torrent and download only the chosen files (numbers from [`Engine::torrent_files`]).
    pub async fn add_torrent(
        &self,
        url: &str,
        folder: Option<&str>,
        queue: Option<i64>,
        files: Vec<u32>,
    ) -> Result<i64, AppError> {
        if files.is_empty() {
            return Err(AppError::Input("choose at least one file".into()));
        }
        self.add_with(url, folder, queue, Some(files)).await
    }

    async fn add_with(
        &self,
        url: &str,
        folder: Option<&str>,
        queue: Option<i64>,
        files: Option<Vec<u32>>,
    ) -> Result<i64, AppError> {
        let url = url.trim();
        if url.is_empty() || url.chars().any(char::is_control) {
            return Err(AppError::Input("type or paste an address".into()));
        }
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("ftp://")
            || lower.starts_with("magnet:?"))
        {
            return Err(AppError::Input(
                "the address must start with http://, https://, ftp:// or magnet:?".into(),
            ));
        }
        let folder = folder.map(str::trim).filter(|f| !f.is_empty());
        match self
            .call(Call::Add {
                url: url.to_string(),
                dest_dir: folder.map(str::to_string),
                cookies: None,
                referrer: None,
                user_agent: None,
                headers: Vec::new(),
                sort: false,
                duplicates: Some("skip".into()),
                queue,
                files,
                video: false,
            })
            .await?
        {
            Response::Added { id } => Ok(id),
            Response::Skipped { reason } => Err(AppError::Input(reason)),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    pub async fn config(&self) -> Result<Config, AppError> {
        match self.call(Call::GetConfig).await? {
            Response::Config {
                download_folder,
                default_folder,
                speed_limit,
                max_active,
                notifications,
                keep_in_tray,
                sort_by_type,
            } => Ok(Config {
                download_folder,
                default_folder,
                speed_limit,
                max_active,
                notifications,
                keep_in_tray,
                sort_by_type,
            }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    pub async fn set_config(&self, change: ConfigChange) -> Result<Config, AppError> {
        match self
            .call(Call::SetConfig {
                download_folder: change.download_folder,
                speed_limit: change.speed_limit,
                max_active: change.max_active,
                notifications: change.notifications,
                keep_in_tray: change.keep_in_tray,
                sort_by_type: change.sort_by_type,
            })
            .await?
        {
            Response::Config {
                download_folder,
                default_folder,
                speed_limit,
                max_active,
                notifications,
                keep_in_tray,
                sort_by_type,
            } => Ok(Config {
                download_folder,
                default_folder,
                speed_limit,
                max_active,
                notifications,
                keep_in_tray,
                sort_by_type,
            }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// The browser extension's settings (shared with the browsers).
    pub async fn extension_settings(&self) -> Result<ExtensionSettings, AppError> {
        let text = match self
            .call(Call::SettingGet {
                key: EXTENSION_KEY.into(),
            })
            .await?
        {
            Response::Setting { value } => value,
            _ => return Err(AppError::Engine("unexpected answer from the engine".into())),
        };
        Ok(ExtensionSettings::from_text(text.as_deref()))
    }

    /// Saves new extension settings. The browsers take them over at their next check.
    pub async fn set_extension_settings(
        &self,
        new: ExtensionSettings,
    ) -> Result<ExtensionSettings, AppError> {
        let current = match self
            .call(Call::SettingGet {
                key: EXTENSION_KEY.into(),
            })
            .await?
        {
            Response::Setting { value } => value,
            _ => return Err(AppError::Engine("unexpected answer from the engine".into())),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let text = merge_extension(current.as_deref(), &new, now);
        self.call(Call::SettingSet {
            key: EXTENSION_KEY.into(),
            value: text.clone(),
        })
        .await?;
        Ok(ExtensionSettings::from_text(Some(&text)))
    }

    pub async fn categories(&self) -> Result<Vec<Category>, AppError> {
        Self::categories_from(self.call(Call::GetCategories).await?)
    }

    pub async fn add_category(
        &self,
        name: &str,
        extensions: Vec<String>,
        folder: Option<String>,
    ) -> Result<Vec<Category>, AppError> {
        Self::categories_from(
            self.call(Call::AddCategory {
                name: name.to_string(),
                extensions,
                folder,
            })
            .await?,
        )
    }

    pub async fn remove_category(&self, id: i64) -> Result<Vec<Category>, AppError> {
        Self::categories_from(self.call(Call::RemoveCategory { id }).await?)
    }

    fn categories_from(response: Response) -> Result<Vec<Category>, AppError> {
        match response {
            Response::Categories { categories } => Ok(categories
                .into_iter()
                .map(|c| Category {
                    id: c.id,
                    name: c.name,
                    extensions: c.extensions,
                    folder: c.folder,
                })
                .collect()),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    pub async fn queues(&self) -> Result<Vec<Queue>, AppError> {
        Self::queues_from(self.call(Call::GetQueues).await?)
    }

    pub async fn add_queue(&self, name: &str) -> Result<Vec<Queue>, AppError> {
        Self::queues_from(self.call(Call::AddQueue { name: name.into() }).await?)
    }

    pub async fn rename_queue(&self, id: i64, name: &str) -> Result<Vec<Queue>, AppError> {
        Self::queues_from(
            self.call(Call::RenameQueue {
                id,
                name: name.into(),
            })
            .await?,
        )
    }

    /// The main queue cannot be removed; the downloads of a removed queue move to it.
    pub async fn remove_queue(&self, id: i64) -> Result<Vec<Queue>, AppError> {
        Self::queues_from(self.call(Call::RemoveQueue { id }).await?)
    }

    pub async fn set_queue(&self, id: i64, change: QueueChange) -> Result<Vec<Queue>, AppError> {
        Self::queues_from(
            self.call(Call::SetQueue {
                id,
                max_active: change.max_active,
                speed_limit: change.speed_limit,
                windows: change
                    .windows
                    .map(|w| w.into_iter().map(Into::into).collect()),
            })
            .await?,
        )
    }

    fn queues_from(response: Response) -> Result<Vec<Queue>, AppError> {
        match response {
            Response::Queues { queues } => Ok(queues
                .into_iter()
                .map(|q| Queue {
                    id: q.id,
                    name: q.name,
                    max_active: q.max_active,
                    speed_limit: q.speed_limit,
                    windows: q.windows.into_iter().map(Into::into).collect(),
                    open_now: q.open_now,
                })
                .collect()),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// The site profiles in the order they are tried.
    /// The outside programs and whether each is found.
    pub async fn tools(&self) -> Result<Vec<ToolInfo>, AppError> {
        match self.call(Call::GetTools).await? {
            Response::Tools { tools } => Ok(tools
                .into_iter()
                .map(|t| ToolInfo {
                    name: t.name,
                    purpose: t.purpose,
                    chosen: t.chosen,
                    found: t.found,
                    page: t.page,
                    windows_page: t.windows_page,
                    winget: t.winget,
                })
                .collect()),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// Chooses where a tool is (none: find it by itself) and returns the new state.
    pub async fn set_tool(
        &self,
        name: String,
        path: Option<String>,
    ) -> Result<Vec<ToolInfo>, AppError> {
        self.call(Call::SetTool { name, path }).await?;
        self.tools().await
    }

    /// How finished torrents are shared: the upload ratio and the minutes to stop at (none: no limit;
    /// neither: not shared).
    pub async fn seeding(&self) -> Result<Seeding, AppError> {
        match self.call(Call::GetSeeding).await? {
            Response::Seeding { ratio, minutes } => Ok(Seeding { ratio, minutes }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    pub async fn set_seeding(&self, new: Seeding) -> Result<Seeding, AppError> {
        self.call(Call::SetSeeding {
            ratio: new.ratio,
            minutes: new.minutes,
        })
        .await?;
        self.seeding().await
    }

    pub async fn torrent_speed(&self) -> Result<TorrentSpeed, AppError> {
        torrent_speed_answer(self.call(Call::GetTorrentSpeed).await?)
    }

    pub async fn set_torrent_speed(&self, new: TorrentSpeed) -> Result<TorrentSpeed, AppError> {
        torrent_speed_answer(
            self.call(Call::SetTorrentSpeed {
                down_kb: new.down_kb,
                up_kb: new.up_kb,
            })
            .await?,
        )
    }

    /// The browser whose sign-in yt-dlp uses for video pages, if one is chosen.
    pub async fn video_browser(&self) -> Result<Option<String>, AppError> {
        match self.call(Call::GetVideoBrowser).await? {
            Response::Setting { value } => Ok(value),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// Chooses that browser (none: no sign-in is used) and returns what is now chosen.
    pub async fn set_video_browser(
        &self,
        browser: Option<String>,
    ) -> Result<Option<String>, AppError> {
        self.call(Call::SetVideoBrowser { browser }).await?;
        self.video_browser().await
    }

    /// Whether Mohona starts when the person signs in to Windows, and whether this system can do that.
    pub fn startup(&self) -> Result<Startup, AppError> {
        let exe = std::env::current_exe().map_err(|e| AppError::Engine(e.to_string()))?;
        Ok(Startup {
            available: cfg!(windows),
            enabled: startup::enabled(&exe),
        })
    }

    pub fn set_startup(&self, on: bool) -> Result<Startup, AppError> {
        let exe = std::env::current_exe().map_err(|e| AppError::Engine(e.to_string()))?;
        startup::set_enabled(&exe, on)?;
        self.startup()
    }

    /// Where new versions are looked for, without asking GitHub.
    pub async fn update(&self) -> Result<UpdateInfo, AppError> {
        update_answer(self.call(Call::GetUpdate).await?)
    }

    /// Chooses the GitHub repository (`owner/name`, empty for none).
    pub async fn set_update_repo(&self, repo: String) -> Result<UpdateInfo, AppError> {
        update_answer(self.call(Call::SetUpdateRepo { repo }).await?)
    }

    /// Asks GitHub whether a newer version exists.
    pub async fn check_update(&self) -> Result<UpdateInfo, AppError> {
        update_answer(self.call(Call::CheckUpdate).await?)
    }

    /// How finished downloads are scanned for viruses.
    pub async fn scan(&self) -> Result<ScanSetting, AppError> {
        scan_answer(self.call(Call::GetScan).await?)
    }

    /// Changes the scan (`off`, `defender` or `custom` with a command) and returns what is now set.
    pub async fn set_scan(&self, mode: String, command: String) -> Result<ScanSetting, AppError> {
        scan_answer(self.call(Call::SetScan { mode, command }).await?)
    }

    pub async fn profiles(&self) -> Result<Vec<SiteProfile>, AppError> {
        match self.call(Call::GetProfiles).await? {
            Response::Profiles { profiles } => Ok(profiles.into_iter().map(Into::into).collect()),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// Replaces the whole list (checked by the engine) and returns what was saved.
    pub async fn set_profiles(
        &self,
        profiles: Vec<SiteProfile>,
    ) -> Result<Vec<SiteProfile>, AppError> {
        let profiles = profiles.into_iter().map(Into::into).collect();
        self.call(Call::SetProfiles { profiles }).await?;
        self.profiles().await
    }

    /// The rules in the order they are tried.
    pub async fn rules(&self) -> Result<Vec<Rule>, AppError> {
        match self.call(Call::GetRules).await? {
            Response::Rules { rules } => Ok(rules.into_iter().map(Into::into).collect()),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// Replaces the whole list (checked by the engine) and returns what was saved.
    /// Conditions the page does not show (the page a link was on) are kept.
    pub async fn set_rules(&self, rules: Vec<Rule>) -> Result<Vec<Rule>, AppError> {
        let old = match self.call(Call::GetRules).await? {
            Response::Rules { rules } => rules,
            _ => Vec::new(),
        };
        let rules = rules
            .into_iter()
            .map(|r| {
                let kept = old
                    .iter()
                    .find(|o| o.id == r.id)
                    .and_then(|o| o.when.referrer_host.clone());
                mohona_daemon::rules::Rule {
                    id: r.id,
                    name: r.name,
                    enabled: r.enabled,
                    when: mohona_daemon::rules::When {
                        host: r.host,
                        extensions: r.extensions,
                        url_contains: r.url_contains,
                        referrer_host: kept,
                    },
                    then: mohona_daemon::rules::Then {
                        folder: r.folder,
                        queue: r.queue,
                        sort: r.sort,
                        block: r.block,
                    },
                }
            })
            .collect();
        self.call(Call::SetRules { rules }).await?;
        self.rules().await
    }

    pub async fn plan(&self) -> Result<Plan, AppError> {
        Self::plan_from(self.call(Call::GetPlan).await?)
    }

    /// Replaces the schedule (an empty list: downloads may run at any time).
    pub async fn set_plan(&self, windows: Vec<PlanWindow>) -> Result<Plan, AppError> {
        let windows = windows.into_iter().map(Into::into).collect();
        Self::plan_from(self.call(Call::SetPlan { windows }).await?)
    }

    /// Arms `sleep`, `hibernate` or `shutdown` for when the downloads are done
    /// (after a countdown that can be cancelled), or disarms with `None`.
    pub async fn set_after_action(
        &self,
        action: Option<String>,
        countdown_seconds: Option<u32>,
    ) -> Result<Plan, AppError> {
        Self::plan_from(
            self.call(Call::SetAfterAction {
                action,
                countdown_seconds,
            })
            .await?,
        )
    }

    pub async fn cancel_action(&self) -> Result<Plan, AppError> {
        Self::plan_from(self.call(Call::CancelAction).await?)
    }

    fn plan_from(response: Response) -> Result<Plan, AppError> {
        match response {
            Response::Plan {
                windows,
                open_now,
                action,
                countdown,
                message,
            } => Ok(Plan {
                windows: windows.into_iter().map(Into::into).collect(),
                open_now,
                action,
                countdown,
                message,
            }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// The file of a finished download, for "Open" and "Show in folder". The
    /// path comes from the engine, never from the page, and the file must
    /// still be there.
    pub async fn finished_file(&self, id: i64) -> Result<PathBuf, AppError> {
        let mut tracker = SpeedTracker::new();
        let rows = self.list(&mut tracker).await?;
        let row = rows
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| AppError::Input("that download is no longer in the list".into()))?;
        if row.status != "completed" {
            return Err(AppError::Input("the download is not finished yet".into()));
        }
        let path = row
            .path
            .map(PathBuf::from)
            .ok_or_else(|| AppError::Input("the engine does not know where the file is".into()))?;
        if !path.is_file() {
            return Err(AppError::Input(format!(
                "the file is no longer there: {}",
                path.display()
            )));
        }
        Ok(path)
    }

    /// Like [`Engine::finished_file`], but refuses programs and scripts: a
    /// download must never be started with one click by mistake. (They can
    /// still be shown in their folder.)
    pub async fn openable_file(&self, id: i64) -> Result<PathBuf, AppError> {
        let path = self.finished_file(id).await?;
        if is_risky(&path) {
            return Err(AppError::Input(
                "for safety, programs and scripts are not opened from here; use \"Show in folder\""
                    .into(),
            ));
        }
        Ok(path)
    }

    /// Stops the engine (running downloads are paused and continue next time).
    pub async fn stop(&self) -> Result<(), AppError> {
        self.call(Call::Shutdown).await.map(|_| ())
    }

    /// Live numbers of one download: its connections and their speed.
    pub async fn details(&self, id: i64) -> Result<Details, AppError> {
        match self.call(Call::GetDetails { id }).await? {
            Response::Details {
                id,
                status,
                downloaded,
                size,
                speed,
                connections,
            } => Ok(Details {
                id,
                status,
                downloaded,
                size,
                speed,
                connections: connections
                    .into_iter()
                    .map(|c| Connection {
                        number: c.number,
                        start: c.start,
                        end: c.end,
                        received: c.received,
                        speed: c.speed,
                        state: c.state,
                    })
                    .collect(),
            }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    pub async fn pause(&self, id: i64) -> Result<(), AppError> {
        self.call(Call::Pause { id }).await.map(|_| ())
    }

    pub async fn resume(&self, id: i64) -> Result<(), AppError> {
        self.call(Call::Resume { id }).await.map(|_| ())
    }

    pub async fn cancel(&self, id: i64) -> Result<(), AppError> {
        self.call(Call::Cancel { id }).await.map(|_| ())
    }

    pub async fn remove(&self, id: i64, delete_file: bool) -> Result<(), AppError> {
        self.call(Call::Remove { id, delete_file })
            .await
            .map(|_| ())
    }
}

/// Starts `mohona daemon` in the background, not tied to the app's window.
fn spawn(
    program: &std::path::Path,
    info: &std::path::Path,
    database: Option<&std::path::Path>,
) -> std::io::Result<()> {
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
        // No console window, and not part of the app's process group.
        command.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    command.spawn().map(|_| ())
}

/// Where the `mohona` program is: the explicit setting, else next to `exe`,
/// else the first folder of `path` that has it.
pub fn find_program(
    setting: Option<std::ffi::OsString>,
    exe: Option<&std::path::Path>,
    path: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "mohona.exe"
    } else {
        "mohona"
    };
    if let Some(p) = setting.map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    if let Some(p) = exe
        .and_then(|e| e.parent())
        .map(|d| d.join(name))
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    std::env::split_paths(&path?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_program_in_the_order_setting_then_next_to_the_app_then_path() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "mohona.exe"
        } else {
            "mohona"
        };
        let (a, b, c) = (
            dir.path().join("a"),
            dir.path().join("b"),
            dir.path().join("c"),
        );
        for d in [&a, &b, &c] {
            std::fs::create_dir(d).unwrap();
        }
        std::fs::write(a.join("custom"), "").unwrap();
        std::fs::write(b.join(name), "").unwrap();
        std::fs::write(c.join(name), "").unwrap();
        let exe = b.join("app");
        let path = std::env::join_paths([&c]).unwrap();
        assert_eq!(
            find_program(
                Some(a.join("custom").into()),
                Some(&exe),
                Some(path.clone())
            ),
            Some(a.join("custom"))
        );
        assert_eq!(
            find_program(
                Some(a.join("missing").into()),
                Some(&exe),
                Some(path.clone())
            ),
            Some(b.join(name))
        );
        assert_eq!(
            find_program(None, Some(&a.join("app")), Some(path)),
            Some(c.join(name))
        );
        assert_eq!(find_program(None, None, None), None);
    }
}

pub use mohona_daemon::openfile::is_risky;

#[cfg(test)]
mod risk_tests {
    use super::is_risky;
    use std::path::Path;

    #[test]
    fn programs_and_scripts_are_risky_documents_and_media_are_not() {
        for bad in [
            "a.exe",
            "A.EXE",
            "setup.msi",
            "x.bat",
            "run.ps1",
            "x.lnk",
            "photo.jpg.exe",
            "a.exe.",
            "a.exe ",
            "m.docm",
            "x.AppImage",
        ] {
            assert!(is_risky(Path::new(bad)), "{bad}");
        }
        for fine in [
            "a.zip",
            "movie.mp4",
            "report.pdf",
            "notes.txt",
            "readme",
            "exe",
            "a.exe.txt",
            ".bashrc2",
        ] {
            assert!(!is_risky(Path::new(fine)), "{fine}");
        }
    }
}

/// Starting with Windows, as the app shows it.
#[derive(Debug, Clone, Serialize)]
pub struct Startup {
    /// This system can do it (Windows only).
    pub available: bool,
    pub enabled: bool,
}

/// The virus scan setting, as the app shows it.
#[derive(Debug, Clone, Serialize)]
pub struct ScanSetting {
    /// `off`, `defender` or `custom`.
    pub mode: String,
    pub command: String,
    /// Where Windows Defender's command-line scanner is, when this computer has it.
    pub defender: Option<String>,
}

fn scan_answer(answer: Response) -> Result<ScanSetting, AppError> {
    match answer {
        Response::Scan {
            mode,
            command,
            defender,
        } => Ok(ScanSetting {
            mode,
            command,
            defender,
        }),
        _ => Err(AppError::Engine("unexpected answer from the engine".into())),
    }
}

/// What the update check knows.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub repo: String,
    pub current: String,
    /// The newest version found; `None` until a check has been made.
    pub latest: Option<String>,
    pub newer: bool,
    pub url: Option<String>,
    pub notes: Option<String>,
}

fn update_answer(answer: Response) -> Result<UpdateInfo, AppError> {
    match answer {
        Response::Update {
            repo,
            current,
            latest,
            newer,
            url,
            notes,
        } => Ok(UpdateInfo {
            repo,
            current,
            latest,
            newer,
            url,
            notes,
        }),
        Response::Error { message } => Err(AppError::Engine(message)),
        _ => Err(AppError::Engine("unexpected answer from the engine".into())),
    }
}

/// Whether the app may open this address for the person: a page of a GitHub repository, over https.
pub fn is_release_page(url: &str, repo: &str) -> bool {
    mohona_daemon::update::valid_repo(repo)
        && url.to_ascii_lowercase().starts_with(&format!(
            "https://github.com/{}/",
            repo.to_ascii_lowercase()
        ))
}

fn torrent_speed_answer(answer: Response) -> Result<TorrentSpeed, AppError> {
    match answer {
        Response::TorrentSpeed { down_kb, up_kb } => Ok(TorrentSpeed { down_kb, up_kb }),
        Response::Error { message } => Err(AppError::Engine(message)),
        _ => Err(AppError::Engine("unexpected answer from the engine".into())),
    }
}
