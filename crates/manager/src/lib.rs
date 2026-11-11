//! Mohona download manager.
//!
//! One long-lived task owns every download. Front ends hold a cheap,
//! cloneable [`Manager`] handle, send it commands (add, pause, resume,
//! cancel, remove) and watch [`ManagerEvent`]s. Every state change is saved
//! in the database first, so a restart picks up where it stopped.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod actions;
mod network;
mod routing;
pub mod scan;
mod schedule;

pub use actions::{ActionRunner, AfterAction, SystemRunner};
pub use mohona_torrent::{
    is_torrent_address, FileEntry as TorrentFile, Listing as TorrentListing,
    Shared as SharedTorrent,
};
pub use network::{NetworkProbe, NetworkState, SystemNetwork};
pub use schedule::{parse_days, Moment, Schedule, Window};

use mohona_core::{
    CancelToken, DownloadRequest, Error as CoreError, Event, Outcome, Progress, ProgressStore,
    RateLimit, RetryPolicy,
};
use mohona_net::{HttpClient, RequestContext};
use mohona_store::{CategoryRow, DownloadRow, ProgressRecord, QueueRow, Status, Store, StoreError};
use tokio::sync::{broadcast, mpsc, oneshot};

pub use mohona_store::{
    CategoryRow as Category, DownloadRow as Download, QueueRow as Queue, Status as DownloadStatus,
};

/// How often the downloaded byte count is written to the database.
const SAVE_INTERVAL: Duration = Duration::from_secs(1);

/// The setting that names the browser whose sign-in yt-dlp uses (empty: none).
pub const VIDEO_BROWSER: &str = "video.browser";
/// The kind of file taken from video pages: quality, video format and audio format (settings).
pub const VIDEO_QUALITY: &str = "video.quality";
pub const VIDEO_CONTAINER: &str = "video.container";
pub const VIDEO_AUDIO: &str = "video.audio";

/// Sharing finished torrents: stop at this upload ratio (settings; empty or 0: no limit) ...
pub const TORRENT_RATIO: &str = "torrent.ratio";
/// Speed limits of all torrents together, in KB/s (settings; empty or 0: no limit).
pub const TORRENT_DOWN_KB: &str = "torrent.down_kb";
pub const TORRENT_UP_KB: &str = "torrent.up_kb";
/// ... or after this many minutes. With neither set, a finished torrent is not shared.
pub const TORRENT_MINUTES: &str = "torrent.minutes";

/// Errors a command can return.
#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    /// The database failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// No download has this id.
    #[error("no download with id {0}")]
    NotFound(i64),
    /// The command makes no sense in the download's current state.
    #[error("cannot do that to a download that is {0}")]
    WrongState(&'static str),
    /// It duplicates something and the policy is to ask.
    #[error("{0}")]
    Duplicate(Duplicate),
    /// The schedule is not valid.
    #[error("{0}")]
    BadSchedule(String),
    /// A queue could not be added, renamed or removed (the reason is the text).
    #[error("{0}")]
    Queue(String),
    /// A fresh link was refused: it could not be checked, or it is not the same file.
    #[error("{0}")]
    Link(String),
    /// A torrent's files could not be listed.
    #[error("{0}")]
    Torrent(String),
    /// The manager has shut down.
    #[error("the download manager has stopped")]
    Stopped,
}

/// Something a front end may want to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagerEvent {
    /// A download was added.
    Added {
        /// Its id.
        id: i64,
    },
    /// A download changed state.
    State {
        /// Its id.
        id: i64,
        /// The new state.
        status: Status,
    },
    /// Bytes received so far.
    Progress {
        /// Its id.
        id: i64,
        /// Bytes on disk.
        downloaded: u64,
        /// Size, if known.
        total: Option<u64>,
    },
    /// Where the parts of a segmented download are (about twice a second).
    Parts {
        /// Its id.
        id: i64,
        /// One entry per connection's part.
        parts: Vec<mohona_core::Part>,
    },
    /// A download failed.
    Error {
        /// Its id.
        id: i64,
        /// What went wrong.
        message: String,
    },
    /// A download failed on the network and will be started again.
    RetryScheduled {
        /// Its id.
        id: i64,
        /// Which automatic retry this is (1 for the first).
        attempt: u32,
        /// How long it waits before starting again.
        wait: Duration,
        /// What went wrong this time.
        message: String,
    },
    /// The after-download action is about to happen; the person can still
    /// cancel it.
    ActionCountdown {
        /// What will happen.
        action: AfterAction,
        /// Seconds until it does.
        seconds_left: u32,
    },
    /// The countdown was stopped (by the person or because there is work again).
    ActionCancelled {
        /// Whether the action stays armed for later.
        still_armed: bool,
    },
    /// The action was carried out. For `Exit` the application should close now.
    ActionDone {
        /// What was done.
        action: AfterAction,
    },
    /// The action could not be carried out.
    ActionFailed {
        /// What went wrong.
        message: String,
    },
    /// The network changed (or went away). Running downloads were stopped
    /// with their progress kept; they start again when it is steady.
    NetworkChanged {
        /// Whether there is any network now.
        online: bool,
    },
    /// The network is steady again and downloads were started.
    NetworkResumed,
    /// A queue's schedule opened or closed.
    QueueWindow {
        /// The queue.
        queue_id: i64,
        /// Whether downloads may run now.
        open: bool,
    },
    /// A download was removed.
    Removed {
        /// Its id.
        id: i64,
    },
}

/// A speed limit the manager can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedTarget {
    /// Applies to everything.
    Global,
    /// Applies to the downloads of one queue (together).
    Queue(i64),
    /// Applies to one download.
    Download(i64),
}

/// The settings key of the global speed limit.
const GLOBAL_SPEED_KEY: &str = "speed_limit";

/// What to do when a download is added that looks like one already there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DuplicatePolicy {
    /// Refuse with [`ManagerError::Duplicate`] and let the person choose.
    #[default]
    Ask,
    /// Add it anyway; the new file gets a number (`name (1).ext`).
    Rename,
    /// Do not add it.
    Skip,
    /// Delete what is there (the earlier download and its file, or the file
    /// with that name) and add the new one.
    Overwrite,
}

/// What an added download duplicates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Duplicate {
    /// The same address is already in the list.
    SameAddress {
        /// The earlier download.
        id: i64,
        /// Its state.
        status: Status,
        /// Where its file is, if it finished.
        path: Option<String>,
    },
    /// A file with the name the address suggests is already in the folder.
    /// The name is only a guess from the address; the server may choose
    /// another.
    FileExists {
        /// The file.
        path: PathBuf,
    },
}

impl std::fmt::Display for Duplicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Duplicate::SameAddress { id, status, .. } => {
                write!(f, "already in the list as #{id} ({})", status.as_str())
            }
            Duplicate::FileExists { path } => write!(f, "{} already exists", path.display()),
        }
    }
}

/// A download to add, with the choices around it.
#[derive(Clone, Debug)]
pub struct AddRequest {
    /// The address.
    pub url: String,
    /// The folder to download to. With `sort_into_categories`, the base
    /// folder the category folders are in.
    pub dest_dir: PathBuf,
    /// Move the finished file into the folder of its category.
    pub sort_into_categories: bool,
    /// What to do about duplicates.
    pub duplicates: DuplicatePolicy,
    /// The queue to put it in; `None` is the first (main) queue.
    pub queue_id: Option<i64>,
    /// Cookies, referrer, user agent and headers to send. Saved with the
    /// download (in the database, in plain text).
    pub context: RequestContext,
}

impl AddRequest {
    /// A plain request: no sorting, and duplicates are refused.
    pub fn new(url: impl Into<String>, dest_dir: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            dest_dir: dest_dir.into(),
            sort_into_categories: false,
            duplicates: DuplicatePolicy::Ask,
            queue_id: None,
            context: RequestContext::default(),
        }
    }
}

/// What came of [`Manager::add_checked`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddResult {
    /// It was added with this id.
    Added(i64),
    /// It was not added, because of this duplicate (policy `Skip`).
    Skipped(Duplicate),
}

/// Whether the same address is in the list, or the file the address
/// suggests is already in the folder it would go to.
pub fn find_duplicate(store: &Store, request: &AddRequest) -> Option<Duplicate> {
    let earlier = store.downloads_with_url(&request.url).ok()?;
    if let Some(row) = earlier
        .into_iter()
        .find(|r| !matches!(r.status, Status::Canceled | Status::Failed))
    {
        return Some(Duplicate::SameAddress {
            id: row.id,
            status: row.status,
            path: row.path,
        });
    }
    let name = routing::guess_filename(&request.url)?;
    let mut folders = vec![request.dest_dir.clone()];
    if request.sort_into_categories {
        if let Ok(categories) = store.categories() {
            if let Some(category) = routing::category_of(&name, &categories) {
                folders.push(routing::folder_of(category, &request.dest_dir));
            }
        }
    }
    folders
        .into_iter()
        .map(|folder| folder.join(&name))
        .find(|path| path.exists())
        .map(|path| Duplicate::FileExists { path })
}

/// What a running download's task has been asked to do when it stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Pause,
    Cancel,
    Shutdown,
    /// The queue's schedule closed.
    Schedule,
    /// The network changed.
    Network,
}

enum Command {
    Add {
        url: String,
        dest_dir: PathBuf,
        queue_id: Option<i64>,
        reply: oneshot::Sender<Result<i64, ManagerError>>,
    },
    Pause(i64, Reply),
    Resume(i64, Reply),
    Cancel(i64, Reply),
    Remove {
        id: i64,
        delete_file: bool,
        reply: Reply,
    },
    List(oneshot::Sender<Result<Vec<DownloadRow>, ManagerError>>),
    /// A download's request details and what is known about its file.
    Details(i64, oneshot::Sender<Result<Details, ManagerError>>),
    TorrentFiles(
        String,
        oneshot::Sender<Result<TorrentListing, ManagerError>>,
    ),
    /// The torrents being shared, and stopping one of them (false: it is not being shared).
    Sharing(oneshot::Sender<Result<Vec<SharedTorrent>, ManagerError>>),
    StopSharing(usize, oneshot::Sender<Result<bool, ManagerError>>),
    GetSetting(
        String,
        oneshot::Sender<Result<Option<String>, ManagerError>>,
    ),
    PutSetting(String, String, oneshot::Sender<Result<(), ManagerError>>),
    /// Give a stopped download a new address and details, and queue it.
    SetLink {
        id: i64,
        url: String,
        context: Box<RequestContext>,
        reply: Reply,
    },
    SetQueueLimit(i64, u32, Reply),
    SetSchedule(i64, Option<Schedule>, Reply),
    AddChecked(
        Box<AddRequest>,
        oneshot::Sender<Result<AddResult, ManagerError>>,
    ),
    Categories(oneshot::Sender<Result<Vec<CategoryRow>, ManagerError>>),
    AddCategory {
        name: String,
        extensions: Vec<String>,
        folder: Option<String>,
        reply: oneshot::Sender<Result<i64, ManagerError>>,
    },
    RemoveCategory(i64, Reply),
    SetAction(Option<(AfterAction, u32)>, Reply),
    /// A tick of the countdown with this generation number.
    CountdownTick(u64),
    Queues(oneshot::Sender<Result<Vec<QueueRow>, ManagerError>>),
    AddQueue(String, oneshot::Sender<Result<i64, ManagerError>>),
    RenameQueue(i64, String, Reply),
    RemoveQueue(i64, Reply),
    /// Which speed limit to change, and to what (`None` removes it).
    SetSpeed(SpeedTarget, Option<u64>, Reply),
    Shutdown(oneshot::Sender<()>),
    /// The virus scan of a finished download is over.
    Scanned(i64, scan::Verdict),
    /// yt-dlp does not know the page: download the address as an ordinary file instead.
    NotAVideo(i64),
    /// Sent by a download's own task when it ends.
    Ended {
        id: i64,
        result: Result<Outcome, CoreError>,
        /// The category the finished file was sorted into.
        category: Option<i64>,
    },
    /// Sent by a download's own task when it learned about the file.
    Started {
        id: i64,
        url: String,
        filename: String,
        total: Option<u64>,
        resumed: u64,
    },
    Saved {
        id: i64,
        downloaded: u64,
    },
    /// A retry timer ran out.
    RetryNow(i64),
}

type Reply = oneshot::Sender<Result<(), ManagerError>>;

/// A download's row, its request details and what is known about its file.
type Details = (DownloadRow, RequestContext, Option<ProgressRecord>);

/// Handle to the download manager. Clone it freely.
#[derive(Clone)]
pub struct Manager {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<ManagerEvent>,
    client: HttpClient,
}

/// Tells the manager what time it is, for schedules. Tests replace it.
#[derive(Clone)]
pub struct Clock(pub Arc<dyn Fn() -> Moment + Send + Sync>);

impl Default for Clock {
    fn default() -> Self {
        Clock(Arc::new(Moment::now))
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Clock")
    }
}

/// What the manager asks about the network. Tests replace it.
#[derive(Clone)]
pub struct Network(pub Arc<dyn NetworkProbe>);

impl Default for Network {
    fn default() -> Self {
        Network(Arc::new(SystemNetwork))
    }
}

impl std::fmt::Debug for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Network")
    }
}

/// The thing that carries out after-download actions. Tests replace it.
#[derive(Clone)]
pub struct Runner(pub Arc<dyn ActionRunner>);

impl Default for Runner {
    fn default() -> Self {
        Runner(Arc::new(SystemRunner))
    }
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Runner")
    }
}

/// Settings for [`Manager::start`].
#[derive(Clone, Debug)]
pub struct ManagerConfig {
    /// Tune each download's connections adaptively (the default). Tests
    /// turn this off.
    pub adaptive: bool,
    /// The most connections one download may use.
    pub connections: usize,
    /// How each download deals with failing requests.
    pub retry: RetryPolicy,
    /// How many times a download that failed on the network is started
    /// again by itself before it is marked failed. The count starts over
    /// whenever an attempt got new data.
    pub max_retries: u32,
    /// The wait before the first automatic retry; each further one doubles
    /// it (up to 5 minutes).
    pub retry_delay: Duration,
    /// The local time, for queue schedules.
    pub clock: Clock,
    /// How often schedules are checked.
    pub schedule_check: Duration,
    /// Carries out after-download actions.
    pub runner: Runner,
    /// The length of one countdown second (only tests change it).
    pub countdown_tick: Duration,
    /// Tells the manager about the network.
    pub network: Network,
    /// How often the network is looked at.
    pub network_check: Duration,
    /// How long the network must stay the same before downloads start again
    /// after a change.
    pub network_settle: Duration,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            adaptive: true,
            connections: mohona_core::DEFAULT_CONNECTIONS,
            retry: RetryPolicy::default(),
            max_retries: 5,
            retry_delay: Duration::from_secs(5),
            clock: Clock::default(),
            schedule_check: Duration::from_secs(10),
            runner: Runner::default(),
            countdown_tick: Duration::from_secs(1),
            network: Network::default(),
            network_check: Duration::from_secs(3),
            network_settle: Duration::from_secs(3),
        }
    }
}

impl Manager {
    /// Start the manager task. Downloads left running by an earlier crash are
    /// put back in their queues and started again (they resume where they
    /// stopped).
    pub fn start(
        store: Store,
        client: HttpClient,
        config: ManagerConfig,
    ) -> Result<Manager, ManagerError> {
        store.reset_interrupted()?;
        let (commands, inbox) = mpsc::channel(256);
        let (events, _) = broadcast::channel(1024);
        let global = store
            .setting(GLOBAL_SPEED_KEY)?
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(0);
        let net = network::Watch::new(config.network.0.state(), std::time::Instant::now());
        let for_checks = client.clone();
        let torrents_dir = store
            .path()
            .and_then(|p| p.parent())
            .map_or_else(std::env::temp_dir, std::path::Path::to_path_buf)
            .join("torrents");
        let actor = Actor {
            store,
            client,
            config,
            inbox,
            commands: commands.clone(),
            events: events.clone(),
            running: HashMap::new(),
            stopping: false,
            retries: HashMap::new(),
            no_video: std::collections::HashSet::new(),
            global_limit: RateLimit::new(global),
            queue_limits: HashMap::new(),
            download_limits: HashMap::new(),
            queue_open: HashMap::new(),
            armed: None,
            finished_since_armed: 0,
            countdown: None,
            generation: 0,
            net,
            torrents: Arc::new(mohona_torrent::Torrents::new(torrents_dir)),
        };
        tokio::spawn(actor.run());
        Ok(Manager {
            commands,
            events,
            client: for_checks,
        })
    }

    /// Watch what the manager does. Slow receivers may miss events; the
    /// database always has the truth.
    pub fn subscribe(&self) -> broadcast::Receiver<ManagerEvent> {
        self.events.subscribe()
    }

    async fn ask<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, ManagerError>>) -> Command,
    ) -> Result<T, ManagerError> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(make(tx))
            .await
            .map_err(|_| ManagerError::Stopped)?;
        rx.await.map_err(|_| ManagerError::Stopped)?
    }

    /// Add a download. It starts as soon as its queue has a free slot.
    pub async fn add(
        &self,
        url: impl Into<String>,
        dest_dir: impl Into<PathBuf>,
    ) -> Result<i64, ManagerError> {
        let (url, dest_dir) = (url.into(), dest_dir.into());
        self.ask(|reply| Command::Add {
            url,
            dest_dir,
            queue_id: None,
            reply,
        })
        .await
    }

    /// Add a download, with sorting into categories and a policy for
    /// duplicates. [`Manager::add`] is the plain form.
    pub async fn add_checked(&self, request: AddRequest) -> Result<AddResult, ManagerError> {
        self.ask(|reply| Command::AddChecked(Box::new(request), reply))
            .await
    }

    /// All categories.
    pub async fn categories(&self) -> Result<Vec<CategoryRow>, ManagerError> {
        self.ask(Command::Categories).await
    }

    /// Add a category (extensions without dots). The folder is absolute or
    /// relative to the folder a download was asked to go to; without one the
    /// category's name is used.
    pub async fn add_category(
        &self,
        name: impl Into<String>,
        extensions: Vec<String>,
        folder: Option<String>,
    ) -> Result<i64, ManagerError> {
        let name = name.into();
        self.ask(|reply| Command::AddCategory {
            name,
            extensions,
            folder,
            reply,
        })
        .await
    }

    /// Remove a category. Files already sorted stay where they are.
    pub async fn remove_category(&self, id: i64) -> Result<(), ManagerError> {
        self.ask(|r| Command::RemoveCategory(id, r)).await
    }

    /// Stop a download but keep what was downloaded.
    pub async fn pause(&self, id: i64) -> Result<(), ManagerError> {
        self.ask(|r| Command::Pause(id, r)).await
    }

    /// Put a paused, failed or canceled download back in its queue.
    pub async fn resume(&self, id: i64) -> Result<(), ManagerError> {
        self.ask(|r| Command::Resume(id, r)).await
    }

    /// Stop a download and delete its partial data.
    pub async fn cancel(&self, id: i64) -> Result<(), ManagerError> {
        self.ask(|r| Command::Cancel(id, r)).await
    }

    /// Remove a download from the list. A running one is cancelled first.
    /// With `delete_file`, a finished file is deleted from disk too.
    pub async fn remove(&self, id: i64, delete_file: bool) -> Result<(), ManagerError> {
        self.ask(|reply| Command::Remove {
            id,
            delete_file,
            reply,
        })
        .await
    }

    /// A value from the settings table (the browser extension's settings, for one).
    pub async fn setting(&self, key: &str) -> Result<Option<String>, ManagerError> {
        self.ask(|r| Command::GetSetting(key.to_string(), r)).await
    }

    /// Saves a value in the settings table.
    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), ManagerError> {
        self.ask(|r| Command::PutSetting(key.to_string(), value.to_string(), r))
            .await
    }

    /// The files inside a torrent (a magnet link first has to find them from peers: up to 90 seconds).
    pub async fn torrent_files(&self, url: &str) -> Result<TorrentListing, ManagerError> {
        self.ask(|r| Command::TorrentFiles(url.to_string(), r))
            .await
    }

    /// The finished torrents that are being shared now.
    pub async fn sharing(&self) -> Result<Vec<SharedTorrent>, ManagerError> {
        self.ask(Command::Sharing).await
    }

    /// Stops sharing one torrent; false when it was not being shared.
    pub async fn stop_sharing(&self, id: usize) -> Result<bool, ManagerError> {
        self.ask(|r| Command::StopSharing(id, r)).await
    }

    /// The request details saved with a download (its source page, cookies, headers).
    pub async fn request_context(&self, id: i64) -> Result<RequestContext, ManagerError> {
        Ok(self.ask(|r| Command::Details(id, r)).await?.1)
    }

    /// Continue a download that failed (or was paused) from a fresh link to
    /// the same file, for example after the old link expired.
    ///
    /// The new link is checked first, and nothing changes if it does not
    /// pass: when the old file's size is known the new link must serve a
    /// file of the same size, and the same name (or, when only the name
    /// differs, the same `ETag` or `Last-Modified`). Whatever was downloaded
    /// already is kept. `update` carries the new cookies, referrer and user
    /// agent; what it leaves out stays as it was.
    pub async fn refresh_link(
        &self,
        id: i64,
        url: &str,
        update: RequestContext,
    ) -> Result<(), ManagerError> {
        let (row, mut context, record) = self.ask(|r| Command::Details(id, r)).await?;
        if !matches!(
            row.status,
            Status::Failed | Status::Paused | Status::Canceled
        ) {
            return Err(ManagerError::WrongState(row.status.as_str()));
        }
        let (url, from_address) = mohona_net::split_userinfo(url.trim());
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(ManagerError::Link(
                "only http and https links can be refreshed".into(),
            ));
        }
        if update.cookies.is_some() {
            context.cookies = update.cookies;
        }
        if update.referrer.is_some() {
            context.referrer = update.referrer;
        }
        if update.user_agent.is_some() {
            context.user_agent = update.user_agent;
        }
        for (name, value) in update.headers {
            context
                .headers
                .retain(|(old, _)| !old.eq_ignore_ascii_case(&name));
            context.headers.push((name, value));
        }
        if from_address.is_some() {
            context.credentials = from_address;
        }
        let scoped = self
            .client
            .with_context(&context)
            .map_err(|e| ManagerError::Link(e.to_string()))?;
        let probe = scoped
            .probe(&url)
            .await
            .map_err(|e| ManagerError::Link(format!("the new link does not work: {e}")))?;
        if let Some(expected) = row.size {
            match probe.size {
                Some(size) if size == expected => {}
                Some(size) => {
                    return Err(ManagerError::Link(format!(
                        "the new link is a different file ({size} bytes, expected {expected})"
                    )))
                }
                None => {
                    return Err(ManagerError::Link(
                        "the new link does not say how big the file is, so it cannot be checked"
                            .into(),
                    ))
                }
            }
            let known_name = row.filename.as_deref().map(mohona_core::sanitize_filename);
            let new_name = probe
                .filename
                .as_deref()
                .map(mohona_core::sanitize_filename);
            let same_name = match (&known_name, &new_name) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                _ => true,
            };
            let agree = |old: &Option<String>, new: &Option<String>| match (old, new) {
                (Some(a), Some(b)) => Some(a == b),
                _ => None,
            };
            let (etag, modified) = record.as_ref().map_or((None, None), |r| {
                (
                    agree(&r.etag, &probe.etag),
                    agree(&r.last_modified, &probe.last_modified),
                )
            });
            if etag == Some(false) || modified == Some(false) {
                return Err(ManagerError::Link(
                    "the new link is a different version of the file (its ETag or date changed)"
                        .into(),
                ));
            }
            if !same_name && etag != Some(true) && modified != Some(true) {
                return Err(ManagerError::Link(format!(
                    "the new link has a different file name ({} instead of {})",
                    new_name.unwrap_or_default(),
                    known_name.unwrap_or_default()
                )));
            }
        }
        self.ask(|reply| Command::SetLink {
            id,
            url,
            context: Box::new(context),
            reply,
        })
        .await
    }

    /// Every download, oldest first.
    pub async fn list(&self) -> Result<Vec<DownloadRow>, ManagerError> {
        self.ask(Command::List).await
    }

    /// How many downloads the queue may run at once.
    pub async fn set_queue_limit(
        &self,
        queue_id: i64,
        max_active: u32,
    ) -> Result<(), ManagerError> {
        self.ask(|r| Command::SetQueueLimit(queue_id, max_active, r))
            .await
    }

    /// Arm an action for when all downloads are finished, or disarm with
    /// `None`. When the work is done, a countdown of `countdown_seconds`
    /// runs (reported as events) before the action; [`Manager::cancel_action`]
    /// stops it. Adding more downloads during the countdown stops it too, and
    /// the action stays armed. The action only waits for downloads that finish
    /// after it was armed, and it is not remembered across restarts.
    pub async fn set_after_action(
        &self,
        action: Option<AfterAction>,
        countdown_seconds: u32,
    ) -> Result<(), ManagerError> {
        self.ask(|r| Command::SetAction(action.map(|a| (a, countdown_seconds)), r))
            .await
    }

    /// Stop a running countdown and disarm the action.
    pub async fn cancel_action(&self) -> Result<(), ManagerError> {
        self.set_after_action(None, 0).await
    }

    /// Give a queue a schedule, or with `None` let it run at any time. Takes
    /// effect at once: a queue whose window is closed stops its downloads
    /// (keeping their progress) and starts them again when it opens.
    pub async fn set_schedule(
        &self,
        queue_id: i64,
        schedule: Option<Schedule>,
    ) -> Result<(), ManagerError> {
        self.ask(|r| Command::SetSchedule(queue_id, schedule, r))
            .await
    }

    /// Add a queue; returns its id.
    pub async fn add_queue(&self, name: impl Into<String>) -> Result<i64, ManagerError> {
        let name = name.into();
        self.ask(|r| Command::AddQueue(name, r)).await
    }

    /// Rename a queue.
    pub async fn rename_queue(&self, id: i64, name: impl Into<String>) -> Result<(), ManagerError> {
        let name = name.into();
        self.ask(|r| Command::RenameQueue(id, name, r)).await
    }

    /// Remove a queue; what it holds moves to the main queue (which cannot be removed).
    pub async fn remove_queue(&self, id: i64) -> Result<(), ManagerError> {
        self.ask(|r| Command::RemoveQueue(id, r)).await
    }

    /// All queues, with their limits and schedules.
    pub async fn queues(&self) -> Result<Vec<QueueRow>, ManagerError> {
        self.ask(Command::Queues).await
    }

    /// Set a speed limit in bytes per second, or remove it with `None`. It
    /// takes effect at once, also for downloads that are running, and is
    /// saved. A download goes at the slowest of the global, queue and
    /// download limits that apply to it.
    pub async fn set_speed_limit(
        &self,
        target: SpeedTarget,
        bytes_per_second: Option<u64>,
    ) -> Result<(), ManagerError> {
        self.ask(|r| Command::SetSpeed(target, bytes_per_second, r))
            .await
    }

    /// Stop everything cleanly, keeping partial data, and end the manager.
    /// Downloads that were running are marked queued again, so they resume
    /// on the next start.
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.commands.send(Command::Shutdown(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
}

struct RunningDownload {
    url: String,
    dest_dir: PathBuf,
    cancel: CancelToken,
    intent: Option<Intent>,
}

struct Actor {
    store: Store,
    client: HttpClient,
    config: ManagerConfig,
    inbox: mpsc::Receiver<Command>,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<ManagerEvent>,
    running: HashMap<i64, RunningDownload>,
    /// Set once shutdown starts: nothing new may begin.
    stopping: bool,
    /// Automatic retries: `(attempts in a row, bytes at the last failure)`.
    retries: HashMap<i64, (u32, u64)>,
    /// Downloads yt-dlp did not know: they are downloaded as ordinary files.
    no_video: std::collections::HashSet<i64>,
    /// The speed limits, created on first use and shared with the downloads.
    global_limit: Arc<RateLimit>,
    queue_limits: HashMap<i64, Arc<RateLimit>>,
    download_limits: HashMap<i64, Arc<RateLimit>>,
    /// Whether each queue's schedule is open (queues not listed are open).
    queue_open: HashMap<i64, bool>,
    /// The armed after-download action and the length of its countdown.
    armed: Option<(AfterAction, u32)>,
    /// Downloads that finished since the action was armed.
    finished_since_armed: u32,
    /// A countdown in progress: its generation number (so old ticks are
    /// ignored) and the seconds left.
    countdown: Option<(u64, u32)>,
    generation: u64,
    net: network::Watch,
    /// BitTorrent downloads; its session starts with the first torrent.
    torrents: Arc<mohona_torrent::Torrents>,
}

impl Actor {
    async fn run(mut self) {
        self.apply_schedules();
        self.pump();
        let mut shutdown: Option<oneshot::Sender<()>> = None;
        let mut check = tokio::time::interval(self.config.schedule_check);
        let mut net_check = tokio::time::interval(self.config.network_check);
        loop {
            tokio::select! {
                command = self.inbox.recv() => {
                    let Some(command) = command else { break };
                    if let Command::Shutdown(done) = command {
                        self.stopping = true;
                        for running in self.running.values_mut() {
                            running.intent = Some(Intent::Shutdown);
                            running.cancel.cancel();
                        }
                        shutdown = Some(done);
                    } else {
                        let may_change_work = !matches!(
                            command,
                            Command::Saved { .. } | Command::CountdownTick(_)
                        );
                        self.handle(command);
                        if may_change_work {
                            self.check_idle();
                        }
                    }
                }
                _ = check.tick() => {
                    if !self.stopping {
                        self.apply_schedules();
                        self.check_idle();
                    }
                }
                _ = net_check.tick() => {
                    if !self.stopping {
                        self.watch_network();
                    }
                }
            }
            if shutdown.is_some() && self.running.is_empty() {
                break;
            }
        }
        if let Some(done) = shutdown {
            let _ = done.send(());
        }
    }

    fn add_checked(&mut self, mut request: AddRequest) -> Result<AddResult, ManagerError> {
        // A login written in the address is kept apart from it.
        let (url, from_address) = mohona_net::split_userinfo(&request.url);
        request.url = url;
        if request.context.credentials.is_none() {
            request.context.credentials = from_address;
        }
        if let Some(queue) = request.queue_id {
            if !self.store.queues()?.iter().any(|q| q.id == queue) {
                return Err(ManagerError::Queue(format!("there is no queue {queue}")));
            }
        }
        let duplicate = find_duplicate(&self.store, &request);
        match (duplicate, request.duplicates) {
            (Some(found), DuplicatePolicy::Ask) => return Err(ManagerError::Duplicate(found)),
            (Some(found), DuplicatePolicy::Skip) => return Ok(AddResult::Skipped(found)),
            (Some(found), DuplicatePolicy::Overwrite) => self.clear_duplicate(&found),
            _ => {}
        }
        let id = self.store.add_download_with(
            request.url.trim(),
            &request.dest_dir.to_string_lossy(),
            request.queue_id,
            request.sort_into_categories,
        )?;
        if !request.context.is_empty() {
            self.store
                .set_headers_json(id, Some(&request.context.to_json()))?;
        }
        let _ = self.store.add_log(id, "info", "added");
        self.emit(ManagerEvent::Added { id });
        self.pump();
        Ok(AddResult::Added(id))
    }

    /// Remove what a duplicate stands for: the earlier download and its file,
    /// or the file itself.
    fn clear_duplicate(&mut self, duplicate: &Duplicate) {
        match duplicate {
            Duplicate::SameAddress { id, .. } => {
                let _ = self.remove(*id, true);
            }
            Duplicate::FileExists { path } => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Arm or disarm the after-download action.
    fn arm(&mut self, action: Option<(AfterAction, u32)>) {
        if self.countdown.take().is_some() {
            self.emit(ManagerEvent::ActionCancelled {
                still_armed: action.is_some(),
            });
        }
        self.armed = action;
        self.finished_since_armed = 0;
    }

    /// Whether anything is still to be done: downloads that run, wait for a
    /// slot, wait for a schedule or wait to retry.
    fn has_work(&self) -> bool {
        !self.running.is_empty()
            || self.store.list_downloads().map_or(true, |rows| {
                rows.iter().any(|r| {
                    r.status == Status::Queued
                        || r.status == Status::Retrying
                        || r.status.is_active()
                })
            })
    }

    /// Start the countdown when everything is done, or stop it when there is
    /// work again. Called after anything that may change either.
    fn check_idle(&mut self) {
        let Some((action, seconds)) = self.armed.clone() else {
            return;
        };
        if self.has_work() {
            if self.countdown.take().is_some() {
                self.emit(ManagerEvent::ActionCancelled { still_armed: true });
            }
            return;
        }
        if self.countdown.is_some() || self.finished_since_armed == 0 || self.stopping {
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        self.countdown = Some((generation, seconds));
        self.emit(ManagerEvent::ActionCountdown {
            action,
            seconds_left: seconds,
        });
        if seconds == 0 {
            self.countdown_tick(generation);
            return;
        }
        let commands = self.commands.clone();
        let tick = self.config.countdown_tick;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tick).await;
                if commands
                    .send(Command::CountdownTick(generation))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    fn countdown_tick(&mut self, generation: u64) {
        let Some((current, left)) = self.countdown else {
            return;
        };
        if current != generation {
            return;
        }
        let Some((action, _)) = self.armed.clone() else {
            self.countdown = None;
            return;
        };
        let left = left.saturating_sub(1);
        if left > 0 {
            self.countdown = Some((current, left));
            self.emit(ManagerEvent::ActionCountdown {
                action,
                seconds_left: left,
            });
            return;
        }
        // Time is up. The action happens once.
        self.countdown = None;
        self.armed = None;
        self.finished_since_armed = 0;
        match self.config.runner.0.run(&action) {
            Ok(()) => self.emit(ManagerEvent::ActionDone { action }),
            Err(message) => self.emit(ManagerEvent::ActionFailed { message }),
        }
    }

    /// Open or close queues according to their schedules, and apply the speed
    /// limit of the window that is open.
    fn apply_schedules(&mut self) {
        let Ok(queues) = self.store.queues() else {
            return;
        };
        let now = (self.config.clock.0)();
        let mut opened = false;
        for queue in queues {
            let schedule = queue
                .schedule_json
                .as_deref()
                .and_then(|json| Schedule::from_json(json).ok());
            let window = schedule.as_ref().and_then(|s| s.open_window(now));
            let open = schedule.is_none() || window.is_some();
            let rate = window
                .and_then(|w| w.speed_limit)
                .or(queue.speed_limit)
                .unwrap_or(0);
            self.queue_limits
                .entry(queue.id)
                .or_insert_with(|| RateLimit::new(rate))
                .set(rate);
            let before = self.queue_open.insert(queue.id, open);
            // The first look is not a change: nothing opened or closed.
            if before.is_some_and(|was| was != open) {
                self.emit(ManagerEvent::QueueWindow {
                    queue_id: queue.id,
                    open,
                });
            }
            if open {
                opened |= before == Some(false);
            } else {
                self.close_queue(queue.id);
            }
        }
        if opened {
            self.pump();
        }
    }

    /// Stop what a queue is running and put what waits to retry back in line.
    /// Look at the network: after a change, stop what is running (it keeps
    /// its progress) and hold everything back until the network is steady.
    fn watch_network(&mut self) {
        let state = self.config.network.0.state();
        let step = self
            .net
            .observe(state, std::time::Instant::now(), self.config.network_settle);
        match step {
            network::Step::Nothing => {}
            network::Step::Changed { online } => {
                for running in self.running.values_mut() {
                    if running.intent.is_none() {
                        running.intent = Some(Intent::Network);
                        running.cancel.cancel();
                    }
                }
                if let Ok(rows) = self.store.list_downloads() {
                    for row in rows.into_iter().filter(|r| r.status == Status::Retrying) {
                        self.set_status(row.id, Status::Queued, None);
                    }
                }
                self.emit(ManagerEvent::NetworkChanged { online });
            }
            network::Step::Resume => {
                self.emit(ManagerEvent::NetworkResumed);
                self.pump();
            }
        }
    }

    fn close_queue(&mut self, queue_id: i64) {
        let Ok(rows) = self.store.list_downloads() else {
            return;
        };
        for row in rows.into_iter().filter(|r| r.queue_id == queue_id) {
            if let Some(running) = self.running.get_mut(&row.id) {
                if running.intent.is_none() {
                    running.intent = Some(Intent::Schedule);
                    running.cancel.cancel();
                }
            } else if row.status == Status::Retrying {
                self.set_status(row.id, Status::Queued, None);
            }
        }
    }

    fn queue_is_open(&self, queue_id: i64) -> bool {
        self.queue_open.get(&queue_id).copied().unwrap_or(true)
    }

    fn emit(&self, event: ManagerEvent) {
        let _ = self.events.send(event);
    }

    fn set_status(&self, id: i64, status: Status, error: Option<&str>) {
        if self.store.set_status(id, status, error).is_ok() {
            self.emit(ManagerEvent::State { id, status });
        }
        if let Some(message) = error {
            self.emit(ManagerEvent::Error {
                id,
                message: message.to_string(),
            });
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Add {
                url,
                dest_dir,
                queue_id,
                reply,
            } => {
                let (url, from_address) = mohona_net::split_userinfo(&url);
                let result = self
                    .store
                    .add_download(url.trim(), &dest_dir.to_string_lossy(), queue_id)
                    .map_err(ManagerError::from);
                if let Ok(id) = result {
                    if let Some(credentials) = from_address {
                        let context = RequestContext {
                            credentials: Some(credentials),
                            ..RequestContext::default()
                        };
                        let _ = self.store.set_headers_json(id, Some(&context.to_json()));
                    }
                    let _ = self.store.add_log(id, "info", "added");
                    self.emit(ManagerEvent::Added { id });
                    self.pump();
                }
                let _ = reply.send(result);
            }
            Command::Pause(id, reply) => {
                let _ = reply.send(self.stop(id, Intent::Pause));
            }
            Command::Cancel(id, reply) => {
                let _ = reply.send(self.stop(id, Intent::Cancel));
            }
            Command::Resume(id, reply) => {
                let result = self.requeue(id);
                if result.is_ok() {
                    self.pump();
                }
                let _ = reply.send(result);
            }
            Command::Remove {
                id,
                delete_file,
                reply,
            } => {
                let _ = reply.send(self.remove(id, delete_file));
            }
            Command::GetSetting(key, reply) => {
                let _ = reply.send(self.store.setting(&key).map_err(ManagerError::from));
            }
            Command::PutSetting(key, value, reply) => {
                let _ = reply.send(
                    self.store
                        .set_setting(&key, &value)
                        .map_err(ManagerError::from),
                );
                if key == TORRENT_RATIO || key == TORRENT_MINUTES {
                    self.apply_seeding();
                }
                if key == TORRENT_DOWN_KB || key == TORRENT_UP_KB {
                    self.apply_torrent_speed();
                }
            }
            Command::Sharing(reply) => {
                let _ = reply.send(Ok(self.torrents.sharing()));
            }
            Command::StopSharing(id, reply) => {
                let _ = reply.send(Ok(self.torrents.stop_sharing(id)));
            }
            Command::TorrentFiles(url, reply) => {
                let torrents = self.torrents.clone();
                tokio::spawn(async move {
                    let cancel = CancelToken::new();
                    let listed = tokio::time::timeout(
                        Duration::from_secs(90),
                        torrents.list(mohona_torrent::Source::Address(url), Vec::new(), &cancel),
                    )
                    .await;
                    let _ = reply.send(match listed {
                        Ok(Ok(listing)) => Ok(listing),
                        Ok(Err(e)) => Err(ManagerError::Torrent(e.to_string())),
                        Err(_) => Err(ManagerError::Torrent(
                            "no peer sent the torrent's file list within 90 seconds".into(),
                        )),
                    });
                });
            }
            Command::Details(id, reply) => {
                let result = self.row(id).and_then(|row| {
                    let context = self
                        .store
                        .headers_json(id)?
                        .and_then(|json| RequestContext::from_json(&json).ok())
                        .unwrap_or_default();
                    let record = self.store.load_progress(id)?;
                    Ok((row, context, record))
                });
                let _ = reply.send(result);
            }
            Command::SetLink {
                id,
                url,
                context,
                reply,
            } => {
                let result = self.set_link(id, &url, &context);
                if result.is_ok() {
                    self.pump();
                }
                let _ = reply.send(result);
            }
            Command::List(reply) => {
                let _ = reply.send(self.store.list_downloads().map_err(Into::into));
            }
            Command::SetQueueLimit(queue_id, max_active, reply) => {
                let result = self
                    .store
                    .set_queue_max_active(queue_id, max_active)
                    .map_err(Into::into);
                self.pump();
                let _ = reply.send(result);
            }
            Command::AddChecked(request, reply) => {
                let _ = reply.send(self.add_checked(*request));
            }
            Command::Categories(reply) => {
                let _ = reply.send(self.store.categories().map_err(Into::into));
            }
            Command::AddCategory {
                name,
                extensions,
                folder,
                reply,
            } => {
                let _ = reply.send(
                    self.store
                        .add_category(&name, &extensions, folder.as_deref())
                        .map_err(Into::into),
                );
            }
            Command::RemoveCategory(id, reply) => {
                let _ = reply.send(self.store.remove_category(id).map_err(Into::into));
            }
            Command::SetAction(action, reply) => {
                self.arm(action);
                let _ = reply.send(Ok(()));
            }
            Command::CountdownTick(generation) => self.countdown_tick(generation),
            Command::SetSchedule(queue_id, schedule, reply) => {
                let _ = reply.send(self.set_schedule(queue_id, schedule));
            }
            Command::Queues(reply) => {
                let _ = reply.send(self.store.queues().map_err(Into::into));
            }
            Command::AddQueue(name, reply) => {
                let _ = reply.send(self.add_queue(&name));
            }
            Command::RenameQueue(id, name, reply) => {
                let _ = reply.send(self.rename_queue(id, &name));
            }
            Command::RemoveQueue(id, reply) => {
                let _ = reply.send(self.remove_queue(id));
            }
            Command::SetSpeed(target, limit, reply) => {
                let _ = reply.send(self.set_speed(target, limit));
            }
            Command::Shutdown(_) => unreachable!("handled in run"),
            Command::Scanned(id, verdict) => self.scanned(id, verdict),
            Command::NotAVideo(id) => {
                // Only if it is still the same run (not cancelled or removed meanwhile).
                if !self.stopping && self.running.contains_key(&id) {
                    if let Ok(row) = self.row(id) {
                        self.no_video.insert(id);
                        self.start(row);
                    }
                }
            }
            Command::Started {
                id,
                url,
                filename,
                total,
                resumed,
            } => {
                if self.running.contains_key(&id) {
                    let _ = self.store.set_started(id, &url, &filename, total, resumed);
                    self.set_status(id, Status::Downloading, None);
                }
            }
            Command::Saved { id, downloaded } => {
                let _ = self.store.set_downloaded(id, downloaded);
            }
            Command::Ended {
                id,
                result,
                category,
            } => self.ended(id, result, category),
            Command::RetryNow(id) => {
                if !self.stopping && !self.running.contains_key(&id) {
                    if let Ok(row) = self.row(id) {
                        if row.status == Status::Retrying {
                            if self.queue_is_open(row.queue_id) && !self.net.holding() {
                                self.start(row);
                            } else {
                                self.set_status(row.id, Status::Queued, None);
                            }
                        }
                    }
                }
            }
        }
    }

    fn check_queue_name(&self, name: &str, except: Option<i64>) -> Result<String, ManagerError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
            return Err(ManagerError::Queue(
                "a queue's name must be 1 to 40 characters".into(),
            ));
        }
        let taken = self
            .store
            .queues()?
            .iter()
            .any(|q| Some(q.id) != except && q.name.eq_ignore_ascii_case(name));
        if taken {
            return Err(ManagerError::Queue(format!(
                "there is already a queue named {name}"
            )));
        }
        Ok(name.to_string())
    }

    fn add_queue(&mut self, name: &str) -> Result<i64, ManagerError> {
        let name = self.check_queue_name(name, None)?;
        Ok(self.store.add_queue(&name)?)
    }

    fn rename_queue(&mut self, id: i64, name: &str) -> Result<(), ManagerError> {
        if !self.store.queues()?.iter().any(|q| q.id == id) {
            return Err(ManagerError::Queue(format!("there is no queue {id}")));
        }
        let name = self.check_queue_name(name, Some(id))?;
        Ok(self.store.rename_queue(id, &name)?)
    }

    /// Removes a queue. What it held goes to the main queue; what it was
    /// running is put back in line first.
    fn remove_queue(&mut self, id: i64) -> Result<(), ManagerError> {
        let queues = self.store.queues()?;
        let Some(first) = queues.first().map(|q| q.id) else {
            return Err(ManagerError::Queue("there is no queue".into()));
        };
        if !queues.iter().any(|q| q.id == id) {
            return Err(ManagerError::Queue(format!("there is no queue {id}")));
        }
        if id == first {
            return Err(ManagerError::Queue(
                "the main queue cannot be removed".into(),
            ));
        }
        self.close_queue(id);
        self.store.remove_queue(id)?;
        self.queue_open.remove(&id);
        self.queue_limits.remove(&id);
        self.pump();
        Ok(())
    }

    fn set_schedule(
        &mut self,
        queue_id: i64,
        schedule: Option<Schedule>,
    ) -> Result<(), ManagerError> {
        if !self.store.queues()?.iter().any(|q| q.id == queue_id) {
            return Err(ManagerError::NotFound(queue_id));
        }
        if let Some(schedule) = &schedule {
            schedule.check().map_err(ManagerError::BadSchedule)?;
        }
        self.store.set_queue_schedule(
            queue_id,
            schedule.as_ref().map(Schedule::to_json).as_deref(),
        )?;
        self.apply_schedules();
        self.pump();
        Ok(())
    }

    fn set_speed(&mut self, target: SpeedTarget, limit: Option<u64>) -> Result<(), ManagerError> {
        let rate = limit.unwrap_or(0);
        match target {
            SpeedTarget::Global => {
                match limit {
                    Some(n) => self.store.set_setting(GLOBAL_SPEED_KEY, &n.to_string())?,
                    None => self.store.remove_setting(GLOBAL_SPEED_KEY)?,
                }
                self.global_limit.set(rate);
            }
            SpeedTarget::Queue(id) => {
                if !self.store.queues()?.iter().any(|q| q.id == id) {
                    return Err(ManagerError::NotFound(id));
                }
                self.store.set_queue_speed_limit(id, limit)?;
                // The queue's open window may have a limit of its own.
                self.apply_schedules();
            }
            SpeedTarget::Download(id) => {
                self.row(id)?;
                self.store.set_download_speed_limit(id, limit)?;
                self.download_limits
                    .entry(id)
                    .or_insert_with(|| RateLimit::new(0))
                    .set(rate);
            }
        }
        Ok(())
    }

    /// The limits that apply to a download, created from what is saved.
    fn limits_for(&mut self, row: &DownloadRow) -> Vec<Arc<RateLimit>> {
        let queue_rate = self
            .store
            .queues()
            .ok()
            .and_then(|queues| queues.into_iter().find(|q| q.id == row.queue_id))
            .and_then(|q| q.speed_limit)
            .unwrap_or(0);
        let queue = self
            .queue_limits
            .entry(row.queue_id)
            .or_insert_with(|| RateLimit::new(queue_rate))
            .clone();
        let own = self
            .download_limits
            .entry(row.id)
            .or_insert_with(|| RateLimit::new(row.speed_limit.unwrap_or(0)))
            .clone();
        vec![self.global_limit.clone(), queue, own]
    }

    fn row(&self, id: i64) -> Result<DownloadRow, ManagerError> {
        self.store.download(id)?.ok_or(ManagerError::NotFound(id))
    }

    /// Pause or cancel. A running download is told to stop and settles when
    /// its task reports back; one that is not running settles right away.
    fn stop(&mut self, id: i64, intent: Intent) -> Result<(), ManagerError> {
        let row = self.row(id)?;
        if let Some(running) = self.running.get_mut(&id) {
            running.intent = Some(intent);
            running.cancel.cancel();
            return Ok(());
        }
        match (row.status, intent) {
            (Status::Queued | Status::Retrying, Intent::Pause) => {
                self.set_status(id, Status::Paused, None);
                Ok(())
            }
            (
                Status::Queued | Status::Retrying | Status::Paused | Status::Failed,
                Intent::Cancel,
            ) => {
                self.discard(&row);
                self.set_status(id, Status::Canceled, None);
                Ok(())
            }
            (status, _) => Err(ManagerError::WrongState(status.as_str())),
        }
    }

    fn requeue(&mut self, id: i64) -> Result<(), ManagerError> {
        let row = self.row(id)?;
        match row.status {
            Status::Paused | Status::Failed | Status::Canceled => {
                self.retries.remove(&id);
                self.set_status(id, Status::Queued, None);
                Ok(())
            }
            status => Err(ManagerError::WrongState(status.as_str())),
        }
    }

    fn set_link(
        &mut self,
        id: i64,
        url: &str,
        context: &RequestContext,
    ) -> Result<(), ManagerError> {
        let row = self.row(id)?;
        if self.running.contains_key(&id)
            || !matches!(
                row.status,
                Status::Failed | Status::Paused | Status::Canceled
            )
        {
            return Err(ManagerError::WrongState(row.status.as_str()));
        }
        self.store.set_url(id, url)?;
        self.store.set_headers_json(id, Some(&context.to_json()))?;
        let _ = self
            .store
            .add_log(id, "info", "continued from a fresh link");
        self.requeue(id)
    }

    fn remove(&mut self, id: i64, delete_file: bool) -> Result<(), ManagerError> {
        let row = self.row(id)?;
        if let Some(running) = self.running.get_mut(&id) {
            // Cancel, and let `ended` finish the job.
            running.intent = Some(Intent::Cancel);
            running.cancel.cancel();
        } else {
            self.discard(&row);
        }
        if delete_file {
            if let Some(path) = &row.path {
                let _ = std::fs::remove_file(path);
            }
        }
        self.download_limits.remove(&id);
        self.store.delete_download(id)?;
        self.emit(ManagerEvent::Removed { id });
        self.pump();
        Ok(())
    }

    fn discard(&self, row: &DownloadRow) {
        if row.status == Status::Completed {
            return;
        }
        let dir = std::path::Path::new(&row.dest_dir);
        if let Ok(Some(record)) = self.store.load_progress(row.id) {
            let mut part = dir
                .join(mohona_core::sanitize_filename(&record.filename))
                .into_os_string();
            part.push(".part");
            let _ = std::fs::remove_file(part);
        }
        let _ = self.store.clear_progress(row.id);
        // Progress kept in a sidecar file by an older version.
        mohona_core::discard_partial(dir, &row.url);
    }

    /// Hands a finished download to the virus scanner, if the settings ask for one.
    fn start_scan(&mut self, id: i64, path: PathBuf) {
        let setting = |key: &str| self.store.setting(key).ok().flatten();
        let scanner = match scan::Scanner::from_settings(
            setting(scan::MODE).as_deref(),
            setting(scan::COMMAND).as_deref(),
        ) {
            Ok(None) => return,
            Ok(Some(s)) => s,
            Err(reason) => {
                let _ = self
                    .store
                    .add_log(id, "warning", &format!("virus scan not run: {reason}"));
                return;
            }
        };
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let verdict = scanner.scan(&path).await;
            let _ = commands.send(Command::Scanned(id, verdict)).await;
        });
    }

    /// Writes down what the scanner said. A problem is shown on the download itself.
    fn scanned(&mut self, id: i64, verdict: scan::Verdict) {
        match verdict {
            scan::Verdict::Clean => {
                let _ = self.store.add_log(id, "info", "virus scan: nothing found");
            }
            scan::Verdict::NotRun(reason) => {
                let _ = self
                    .store
                    .add_log(id, "warning", &format!("virus scan not run: {reason}"));
            }
            scan::Verdict::Threat(said) => {
                let _ = self
                    .store
                    .add_log(id, "error", &format!("virus scan: {said}"));
                if matches!(self.store.download(id), Ok(Some(row)) if row.status == Status::Completed)
                {
                    let _ = self.store.set_status(
                        id,
                        Status::Completed,
                        Some(&format!("Virus scan: {said}")),
                    );
                    self.emit(ManagerEvent::State {
                        id,
                        status: Status::Completed,
                    });
                }
            }
        }
    }

    fn ended(&mut self, id: i64, result: Result<Outcome, CoreError>, category: Option<i64>) {
        let running = self.running.remove(&id);
        let intent = running.as_ref().and_then(|r| r.intent);
        // The row is gone when the download was removed while it ran: only
        // its partial data is left to clean up.
        let Ok(Some(row)) = self.store.download(id) else {
            if let (Some(running), Some(Intent::Cancel)) = (running, intent) {
                mohona_core::discard_partial(&running.dest_dir, &running.url);
            }
            self.pump();
            return;
        };
        match (result, intent) {
            (Ok(outcome), _) => {
                self.retries.remove(&id);
                self.finished_since_armed += 1;
                let _ =
                    self.store
                        .set_completed(id, &outcome.path.to_string_lossy(), outcome.bytes);
                if category.is_some() {
                    let _ = self.store.set_category(id, category);
                }
                let _ = self.store.add_log(
                    id,
                    "info",
                    &format!(
                        "completed: {} ({} bytes){}",
                        outcome.path.display(),
                        outcome.bytes,
                        outcome
                            .verified
                            .as_ref()
                            .map_or_else(String::new, |v| format!(
                                ", {} checksum matched",
                                v.checksum.algorithm.name()
                            ))
                    ),
                );
                self.emit(ManagerEvent::State {
                    id,
                    status: Status::Completed,
                });
                self.start_scan(id, outcome.path.clone());
            }
            (Err(_), Some(Intent::Pause)) => self.set_status(id, Status::Paused, None),
            (Err(_), Some(Intent::Schedule | Intent::Network)) => {
                // The queue's window closed: wait for it to open again.
                self.set_status(id, Status::Queued, None);
            }
            (Err(_), Some(Intent::Shutdown)) => {
                // Not a state the user chose: run again at the next start.
                let _ = self.store.set_status(id, Status::Queued, None);
            }
            (Err(_), Some(Intent::Cancel)) => {
                self.discard(&row);
                self.set_status(id, Status::Canceled, None);
            }
            (Err(error), None) => {
                if is_temporary(&error) && self.schedule_retry(&row, &describe(&error)) {
                    // A timer will start it again.
                } else {
                    self.retries.remove(&id);
                    let message = describe(&error);
                    let _ = self
                        .store
                        .add_log(id, "error", &format!("failed: {message}"));
                    self.set_status(id, Status::Failed, Some(&message));
                }
            }
        }
        self.pump();
    }

    /// Wait and try again, if the download has retries left. The download
    /// keeps its place in the queue while it waits.
    fn schedule_retry(&mut self, row: &DownloadRow, message: &str) -> bool {
        if self.stopping {
            return false;
        }
        let entry = self.retries.entry(row.id).or_insert((0, row.downloaded));
        if row.downloaded > entry.1 {
            // It got somewhere since the last failure: start counting over.
            *entry = (0, row.downloaded);
        }
        if entry.0 >= self.config.max_retries {
            return false;
        }
        entry.0 += 1;
        let wait = self
            .config
            .retry_delay
            .saturating_mul(1u32 << (entry.0 - 1).min(16))
            .min(Duration::from_secs(300));
        let attempt = entry.0;
        let _ = self.store.add_log(
            row.id,
            "warn",
            &format!("retry {attempt} in {} s: {message}", wait.as_secs()),
        );
        self.set_status(row.id, Status::Retrying, None);
        self.emit(ManagerEvent::RetryScheduled {
            id: row.id,
            attempt,
            wait,
            message: message.to_string(),
        });
        let commands = self.commands.clone();
        let id = row.id;
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = commands.send(Command::RetryNow(id)).await;
        });
        true
    }

    /// Start whatever the queues have room for.
    fn pump(&mut self) {
        if self.stopping || self.net.holding() {
            return;
        }
        let Ok(ids) = self.store.next_to_start() else {
            return;
        };
        for id in ids {
            if self.running.contains_key(&id) {
                continue;
            }
            let Ok(row) = self.row(id) else { continue };
            if self.queue_is_open(row.queue_id) {
                self.start(row);
            }
        }
    }

    fn start(&mut self, row: DownloadRow) {
        let cancel = CancelToken::new();
        self.running.insert(
            row.id,
            RunningDownload {
                url: row.url.clone(),
                dest_dir: PathBuf::from(&row.dest_dir),
                cancel: cancel.clone(),
                intent: None,
            },
        );
        self.set_status(row.id, Status::Probing, None);
        if mohona_torrent::is_torrent_address(&row.url) {
            self.apply_seeding();
            self.apply_torrent_speed();
            let route = row
                .auto_category
                .then(|| self.store.categories().ok())
                .flatten()
                .map(|categories| (PathBuf::from(&row.dest_dir), categories));
            let only = self
                .store
                .headers_json(row.id)
                .ok()
                .flatten()
                .and_then(|json| RequestContext::from_json(&json).ok())
                .and_then(|c| c.torrent_files);
            tokio::spawn(run_torrent(
                self.torrents.clone(),
                row.url.clone(),
                only,
                PathBuf::from(&row.dest_dir),
                cancel,
                row.id,
                self.commands.clone(),
                self.events.clone(),
                route,
            ));
            return;
        }
        let context = self
            .store
            .headers_json(row.id)
            .ok()
            .flatten()
            .and_then(|json| RequestContext::from_json(&json).ok())
            .unwrap_or_default();
        let web = row.url.starts_with("http://") || row.url.starts_with("https://");
        let tool = |tool: mohona_media::tools::Tool| {
            let chosen = self
                .store
                .setting(&format!("tools.{}", tool.program))
                .ok()
                .flatten()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty());
            tool.find(chosen.as_deref().map(std::path::Path::new))
        };
        // A link to one video of a known site, or a page the person asked for the video of, must
        // be yt-dlp's; any other page is tried with yt-dlp (when there is one) and, when it does
        // not know the page, downloaded as an ordinary file.
        let sure = mohona_media::ytdlp::is_site_page(&row.url) || (context.video && web);
        let ytdlp = tool(mohona_media::tools::YTDLP);
        let guess = !sure
            && ytdlp.is_some()
            && !self.no_video.contains(&row.id)
            && mohona_media::ytdlp::could_be_page(&row.url);
        if sure || guess {
            let route = row
                .auto_category
                .then(|| self.store.categories().ok())
                .flatten()
                .map(|categories| (PathBuf::from(&row.dest_dir), categories));
            let browser = self
                .store
                .setting(VIDEO_BROWSER)
                .ok()
                .flatten()
                .map(|b| b.trim().to_string())
                .filter(|b| !b.is_empty());
            let saved = |key: &str| self.store.setting(key).ok().flatten().unwrap_or_default();
            // A choice made for this one download beats the saved settings.
            let choice = match &context.video_choice {
                Some(c) => {
                    mohona_media::ytdlp::Choice::from_settings(&c.quality, &c.container, &c.audio)
                }
                None => mohona_media::ytdlp::Choice::from_settings(
                    &saved(VIDEO_QUALITY),
                    &saved(VIDEO_CONTAINER),
                    &saved(VIDEO_AUDIO),
                ),
            };
            tokio::spawn(run_video(
                choice,
                ytdlp,
                tool(mohona_media::tools::FFMPEG),
                VideoJob {
                    url: row.url.clone(),
                    dest: PathBuf::from(&row.dest_dir),
                    browser,
                    cookie_header: context.cookies.clone(),
                    guess,
                },
                cancel,
                row.id,
                self.commands.clone(),
                self.events.clone(),
                route,
            ));
            return;
        }
        self.start_http(row, cancel);
    }

    /// Tells the torrent engine how long to share finished torrents (the two settings).
    fn apply_seeding(&self) {
        let number = |key: &str| {
            self.store
                .setting(key)
                .ok()
                .flatten()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
        };
        self.torrents.set_seeding(mohona_torrent::Seeding {
            ratio: number(TORRENT_RATIO),
            minutes: number(TORRENT_MINUTES).map(|m| m as u64),
        });
    }

    /// Tells the torrent engine the speed limits (the two settings, in KB/s).
    fn apply_torrent_speed(&self) {
        let bps = |key: &str| {
            self.store
                .setting(key)
                .ok()
                .flatten()
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|v| *v > 0)
                .map(|kb| kb.saturating_mul(1024))
        };
        self.torrents
            .set_speed(bps(TORRENT_DOWN_KB), bps(TORRENT_UP_KB));
    }

    /// The ordinary download of an address.
    fn start_http(&mut self, row: DownloadRow, cancel: CancelToken) {
        let mut request = DownloadRequest::new(row.url.clone(), row.dest_dir.clone());
        request.connections = self.config.connections;
        request.retry = self.config.retry.clone();
        request.retry.adaptive = self.config.adaptive;
        if let Some(path) = self.store.path() {
            request.progress = Some(Arc::new(DbProgress {
                id: row.id,
                url: row.url.clone(),
                path: path.to_path_buf(),
                connection: Mutex::new(None),
            }));
        }
        request.cancel = cancel;
        request.limits = self.limits_for(&row);
        if let Ok(Some(json)) = self.store.headers_json(row.id) {
            match RequestContext::from_json(&json) {
                Ok(context) => {
                    if let Some(n) = context.connections {
                        request.connections = (n as usize).clamp(1, 32);
                    }
                    request.context = context;
                }
                Err(error) => {
                    let _ = self.store.add_log(
                        row.id,
                        "warn",
                        &format!("ignoring saved headers: {error}"),
                    );
                }
            }
        }
        let route = row
            .auto_category
            .then(|| self.store.categories().ok())
            .flatten()
            .map(|categories| (PathBuf::from(&row.dest_dir), categories));
        tokio::spawn(run_download(
            Arc::new(self.client.clone()),
            request,
            row.id,
            self.commands.clone(),
            self.events.clone(),
            route,
        ));
    }
}

/// A torrent: the same story as `run_download`, told by the torrent engine.
#[allow(clippy::too_many_arguments)]
async fn run_torrent(
    torrents: Arc<mohona_torrent::Torrents>,
    url: String,
    only: Option<Vec<u32>>,
    dest: PathBuf,
    cancel: CancelToken,
    id: i64,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<ManagerEvent>,
    route: Option<(PathBuf, Vec<CategoryRow>)>,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<mohona_torrent::Update>();
    let engine = {
        let url = url.clone();
        let cancel = cancel.clone();
        async move {
            torrents
                .run(
                    mohona_torrent::Source::Address(url),
                    &dest,
                    Vec::new(),
                    only,
                    &cancel,
                    move |update| {
                        let _ = tx.send(update);
                    },
                )
                .await
        }
    };
    tokio::pin!(engine);
    let mut last_save = Instant::now();
    let mut latest = 0u64;
    let mut started = false;
    let result = loop {
        tokio::select! {
            result = &mut engine => break result,
            Some(update) = rx.recv() => {
                if !started {
                    if let Some(name) = &update.name {
                        started = true;
                        let _ = commands
                            .send(Command::Started {
                                id,
                                url: url.clone(),
                                filename: name.clone(),
                                total: update.total,
                                resumed: 0,
                            })
                            .await;
                    }
                }
                if started {
                    latest = update.downloaded;
                    let _ = events.send(ManagerEvent::Progress { id, downloaded: update.downloaded, total: update.total });
                    if last_save.elapsed() >= SAVE_INTERVAL {
                        last_save = Instant::now();
                        let _ = commands.send(Command::Saved { id, downloaded: update.downloaded }).await;
                    }
                }
            }
        }
    };
    if result.is_err() && latest > 0 {
        let _ = commands
            .send(Command::Saved {
                id,
                downloaded: latest,
            })
            .await;
    }
    let result = match result {
        Ok(done) => Ok(Outcome {
            path: done.path,
            bytes: done.bytes,
            final_url: url,
            connections: 0,
            verified: None,
        }),
        Err(mohona_torrent::TorrentError::Cancelled) => Err(CoreError::Cancelled),
        Err(mohona_torrent::TorrentError::Failed(reason)) => Err(CoreError::Stream(reason)),
    };
    // A one-file torrent can be sorted into its category's folder; a folder stays as it is.
    let mut category = None;
    let result = match (result, route) {
        (Ok(mut outcome), Some((base, categories))) if outcome.path.is_file() => {
            let path = outcome.path.clone();
            let sorted =
                tokio::task::spawn_blocking(move || routing::sort_file(&path, &base, &categories))
                    .await;
            if let Ok(Ok((new_path, found))) = sorted {
                outcome.path = new_path;
                category = found;
            }
            Ok(outcome)
        }
        (other, _) => other,
    };
    let _ = commands
        .send(Command::Ended {
            id,
            result,
            category,
        })
        .await;
}

/// Removes a file when dropped (the cookie file, on every way out of a video download).
struct RemoveOnDrop(Option<PathBuf>);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Writes a file only its owner can read.
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_bytes())
}

/// What yt-dlp is to fetch.
struct VideoJob {
    url: String,
    dest: PathBuf,
    /// The browser whose sign-in to use.
    browser: Option<String>,
    /// The `Cookie` header of the page the link came from.
    cookie_header: Option<String>,
    /// Only a guess that the page holds a video: when yt-dlp fails before it starts, the address
    /// is downloaded as an ordinary file instead.
    guess: bool,
}

/// A video page (YouTube and alike): yt-dlp does the work, and tells the same story through its output.
#[allow(clippy::too_many_arguments)]
async fn run_video(
    choice: mohona_media::ytdlp::Choice,
    ytdlp: Option<PathBuf>,
    ffmpeg: Option<PathBuf>,
    job: VideoJob,
    cancel: CancelToken,
    id: i64,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<ManagerEvent>,
    route: Option<(PathBuf, Vec<CategoryRow>)>,
) {
    use mohona_media::ytdlp::{arguments, failure_reason, parse_line, Line, Request};
    use tokio::io::{AsyncBufReadExt, BufReader};

    let has_ffmpeg = ffmpeg.is_some();

    let VideoJob {
        url,
        dest,
        browser,
        cookie_header,
        guess,
    } = job;

    let end = |result: Result<Outcome, CoreError>, category: Option<i64>| {
        let commands = commands.clone();
        async move {
            let _ = commands
                .send(Command::Ended {
                    id,
                    result,
                    category,
                })
                .await;
        }
    };
    let Some(ytdlp) = ytdlp else {
        end(
            Err(CoreError::Stream(
                mohona_media::tools::YTDLP.missing_message(),
            )),
            None,
        )
        .await;
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dest) {
        end(
            Err(CoreError::Stream(format!("cannot use the folder: {e}"))),
            None,
        )
        .await;
        return;
    }
    // The page's sign-in goes to yt-dlp in a cookie file that lives only as long as the run.
    let cookie_file = cookie_header
        .as_deref()
        .and_then(|header| mohona_media::ytdlp::cookie_file_text(&url, header))
        .and_then(|text| {
            let path = std::env::temp_dir()
                .join(format!("mohona-cookies-{}-{id}.txt", std::process::id()));
            write_private(&path, &text).ok().map(|()| path)
        });
    let _cleanup = RemoveOnDrop(cookie_file.clone());
    let mut command = tokio::process::Command::new(&ytdlp);
    command
        .args(arguments(&Request {
            choice,
            url: url.clone(),
            folder: dest.clone(),
            ffmpeg,
            browser,
            cookie_file,
            generic: !guess,
        }))
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // no console window
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            end(
                Err(CoreError::Stream(format!(
                    "could not start {}: {e}",
                    ytdlp.display()
                ))),
                None,
            )
            .await;
            return;
        }
    };
    let (tx, mut lines) = mpsc::unbounded_channel::<String>();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let mut tail: Vec<String> = Vec::new();
    let mut started = false;
    let mut file: Option<PathBuf> = None;
    // A video and its sound are two downloads in a row: the bytes of finished parts are kept.
    let (mut base, mut part, mut part_total) = (0u64, 0u64, 0u64);
    let mut last_save = Instant::now();
    let mut latest = 0u64;
    let stopped = loop {
        tokio::select! {
            () = cancel.cancelled() => break true,
            line = lines.recv() => {
                let Some(line) = line else { break false };
                match parse_line(&line) {
                    Line::Name(name) => {
                        if !started {
                            started = true;
                            let _ = commands
                                .send(Command::Started { id, url: url.clone(), filename: name, total: None, resumed: 0 })
                                .await;
                        }
                    }
                    Line::Progress { downloaded, total } => {
                        if downloaded + 1024 < part {
                            base += part_total.max(part);
                        }
                        part = downloaded;
                        part_total = total.unwrap_or(0);
                        latest = base + downloaded;
                        if started {
                            let _ = events.send(ManagerEvent::Progress { id, downloaded: latest, total: None });
                            if last_save.elapsed() >= SAVE_INTERVAL {
                                last_save = Instant::now();
                                let _ = commands.send(Command::Saved { id, downloaded: latest }).await;
                            }
                        }
                    }
                    Line::File(path) => file = Some(path),
                    Line::Other(text) => {
                        if !text.is_empty() {
                            tail.push(text);
                            if tail.len() > 40 {
                                tail.remove(0);
                            }
                        }
                    }
                }
            }
        }
    };
    if stopped {
        let _ = child.kill().await;
        if latest > 0 {
            let _ = commands
                .send(Command::Saved {
                    id,
                    downloaded: latest,
                })
                .await;
        }
        end(Err(CoreError::Cancelled), None).await;
        return;
    }
    let status = child.wait().await;
    if guess && !started && file.is_none() {
        // Not a video page after all (or yt-dlp could not run): an ordinary download.
        let _ = commands.send(Command::NotAVideo(id)).await;
        return;
    }
    let result = match (status, file) {
        (Ok(s), Some(path)) if s.success() => {
            let bytes = std::fs::metadata(&path).map_or(latest, |m| m.len());
            Ok(Outcome {
                path,
                bytes,
                final_url: url,
                connections: 1,
                verified: None,
            })
        }
        (Ok(s), None) if s.success() => Err(CoreError::Stream(
            "yt-dlp finished but did not say which file it saved".into(),
        )),
        _ => Err(CoreError::Stream(failure_reason(&tail, has_ffmpeg))),
    };
    let mut category = None;
    let result = match (result, route) {
        (Ok(mut outcome), Some((base, categories))) if outcome.path.is_file() => {
            let path = outcome.path.clone();
            let sorted =
                tokio::task::spawn_blocking(move || routing::sort_file(&path, &base, &categories))
                    .await;
            if let Ok(Ok((new_path, found))) = sorted {
                outcome.path = new_path;
                category = found;
            }
            Ok(outcome)
        }
        (other, _) => other,
    };
    end(result, category).await;
}

async fn run_download(
    client: Arc<HttpClient>,
    request: DownloadRequest,
    id: i64,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<ManagerEvent>,
    route: Option<(PathBuf, Vec<CategoryRow>)>,
) {
    let (tx, mut rx) = mpsc::channel::<Event>(64);
    let engine = {
        let client = client.clone();
        let request = request.clone();
        async move { mohona_core::download(&client, &request, Some(&tx)).await }
    };
    tokio::pin!(engine);
    let mut last_save = Instant::now();
    let mut latest = 0u64;
    let result = loop {
        tokio::select! {
            result = &mut engine => break result,
            Some(event) = rx.recv() => match event {
                Event::Started { url, filename, total, resumed, .. } => {
                    latest = resumed;
                    let _ = commands
                        .send(Command::Started { id, url, filename, total, resumed })
                        .await;
                }
                Event::Progress { downloaded, total } => {
                    latest = downloaded;
                    let _ = events.send(ManagerEvent::Progress { id, downloaded, total });
                    if last_save.elapsed() >= SAVE_INTERVAL {
                        last_save = Instant::now();
                        let _ = commands.send(Command::Saved { id, downloaded }).await;
                    }
                }
                Event::Parts(parts) => {
                    let _ = events.send(ManagerEvent::Parts { id, parts });
                }
                _ => {}
            },
        }
    };
    if result.is_err() && latest > 0 {
        let _ = commands
            .send(Command::Saved {
                id,
                downloaded: latest,
            })
            .await;
    }
    // A finished file goes to its category's folder. If that fails the file
    // stays where it is, which is still a finished download.
    let mut category = None;
    let result = match (result, route) {
        (Ok(mut outcome), Some((base, categories))) => {
            let path = outcome.path.clone();
            let sorted =
                tokio::task::spawn_blocking(move || routing::sort_file(&path, &base, &categories))
                    .await;
            if let Ok(Ok((new_path, found))) = sorted {
                outcome.path = new_path;
                category = found;
            }
            Ok(outcome)
        }
        (other, _) => other,
    };
    let _ = commands
        .send(Command::Ended {
            id,
            result,
            category,
        })
        .await;
}

/// An error with its causes, as one line.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        let part = inner.to_string();
        if !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = inner.source();
    }
    text
}

/// Whether trying again later may help.
fn is_temporary(error: &CoreError) -> bool {
    match error {
        CoreError::Net(net) => net.failure() != mohona_net::Failure::Fatal,
        CoreError::Incomplete { .. } => true,
        CoreError::Io(_)
        | CoreError::Cancelled
        | CoreError::Stream(_)
        | CoreError::ChecksumMismatch { .. } => false,
    }
}

/// A download's progress, kept in the database. It has its own connection
/// (opened on first use): the workers save from other threads while the
/// manager task uses its own.
#[derive(Debug)]
struct DbProgress {
    id: i64,
    url: String,
    path: PathBuf,
    connection: Mutex<Option<Store>>,
}

impl DbProgress {
    fn with<T>(&self, work: impl FnOnce(&Store) -> Result<T, StoreError>) -> std::io::Result<T> {
        let mut guard = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some(Store::open(&self.path).map_err(std::io::Error::other)?);
        }
        work(guard.as_ref().expect("opened above")).map_err(std::io::Error::other)
    }
}

impl ProgressStore for DbProgress {
    fn load(&self) -> Option<Progress> {
        let record = self.with(|s| s.load_progress(self.id)).ok()??;
        Some(Progress {
            version: 1,
            url: self.url.clone(),
            final_url: record.final_url,
            filename: record.filename,
            total: record.total,
            etag: record.etag,
            last_modified: record.last_modified,
            segments: record.segments,
        })
    }

    fn save(&self, progress: &Progress) -> std::io::Result<()> {
        let record = ProgressRecord {
            final_url: progress.final_url.clone(),
            filename: progress.filename.clone(),
            total: progress.total,
            etag: progress.etag.clone(),
            last_modified: progress.last_modified.clone(),
            segments: progress.segments.clone(),
        };
        self.with(|s| s.save_progress(self.id, &record))
    }

    fn clear(&self) {
        let _ = self.with(|s| s.clear_progress(self.id));
    }
}
