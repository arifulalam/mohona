use mohona_manager::Window;
use serde::{Deserialize, Serialize};

/// What a client asks. One per line, as JSON: `{"token":"...","call":{"type":"ping"}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub token: String,
    pub call: Call,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Call {
    /// Is the daemon there? Answered with [`Response::Pong`].
    Ping,
    /// What a running download is doing now: its connections and their speed.
    GetDetails {
        id: i64,
    },
    /// A saved setting of the browser extension (key `extension.*`).
    SettingGet {
        key: String,
    },
    /// Save a setting of the browser extension (key `extension.*`, at most 64 KiB).
    SettingSet {
        key: String,
        value: String,
    },
    /// The engine's own settings (for the app).
    GetConfig,
    /// Change some of the engine's settings; what is left out stays.
    SetConfig {
        /// A folder (absolute path); an empty text goes back to the default.
        #[serde(default)]
        download_folder: Option<String>,
        /// Bytes per second for all downloads together; 0 removes the limit.
        #[serde(default)]
        speed_limit: Option<u64>,
        /// How many downloads run at once (1 to 32).
        #[serde(default)]
        max_active: Option<u32>,
        /// Tell the person when a download finishes or fails (the app does it).
        #[serde(default)]
        notifications: Option<bool>,
        /// Closing the app's window leaves it in the tray.
        #[serde(default)]
        keep_in_tray: Option<bool>,
        /// Move finished downloads into the folder of their category (also for the browser's).
        #[serde(default)]
        sort_by_type: Option<bool>,
    },
    /// The default queue's schedule and the armed "when everything is done" action.
    GetPlan,
    /// Give the default queue a schedule (an empty list: it runs at any time).
    SetPlan {
        windows: Vec<Window>,
    },
    /// Arm an action for when the downloads are finished (`sleep`,
    /// `hibernate` or `shutdown`), or disarm with `None`. A countdown (default
    /// 60 seconds, 5 to 3600) runs first and can be cancelled.
    SetAfterAction {
        action: Option<String>,
        #[serde(default)]
        countdown_seconds: Option<u32>,
    },
    /// Stop a running countdown and disarm the action.
    CancelAction,
    /// The categories that sort finished downloads into folders by file type.
    GetCategories,
    AddCategory {
        name: String,
        /// File types without dots, such as `["zip", "7z"]`.
        extensions: Vec<String>,
        /// A full path, or a folder name inside the download folder; empty uses the name.
        #[serde(default)]
        folder: Option<String>,
    },
    RemoveCategory {
        id: i64,
    },
    /// Every queue, with its limits and schedule.
    GetQueues,
    /// A new queue (a unique name of 1 to 40 characters).
    AddQueue {
        name: String,
    },
    RenameQueue {
        id: i64,
        name: String,
    },
    /// Remove a queue; its downloads move to the main queue, which cannot be removed.
    RemoveQueue {
        id: i64,
    },
    /// Change a queue; what is left out stays.
    SetQueue {
        id: i64,
        /// How many of its downloads run at once (1 to 32).
        #[serde(default)]
        max_active: Option<u32>,
        /// Bytes per second for the queue together; 0 removes the limit.
        #[serde(default)]
        speed_limit: Option<u64>,
        /// The times it may run; an empty list means any time.
        #[serde(default)]
        windows: Option<Vec<Window>>,
    },
    /// Add a download. It starts when its queue has a free slot.
    Add {
        url: String,
        /// Folder to save in; default: the daemon's download folder.
        #[serde(default)]
        dest_dir: Option<String>,
        /// The `Cookie` header value of the browser session.
        #[serde(default)]
        cookies: Option<String>,
        /// The page the link was on.
        #[serde(default)]
        referrer: Option<String>,
        #[serde(default)]
        user_agent: Option<String>,
        #[serde(default)]
        headers: Vec<(String, String)>,
        /// Move the finished file into its category's folder.
        #[serde(default)]
        sort: bool,
        /// `ask` (default), `skip` or `rename`.
        #[serde(default)]
        duplicates: Option<String>,
        /// The queue to put it in; default: the main queue.
        #[serde(default)]
        queue: Option<i64>,
    },
    /// Download an HLS or DASH stream by running `mohona stream` for it.
    Stream {
        url: String,
        #[serde(default)]
        dest_dir: Option<String>,
        #[serde(default)]
        cookies: Option<String>,
        #[serde(default)]
        referrer: Option<String>,
        #[serde(default)]
        user_agent: Option<String>,
        #[serde(default)]
        headers: Vec<(String, String)>,
        /// The file name to save as (without extension).
        #[serde(default)]
        name: Option<String>,
    },
    /// Start receiving a file the browser holds (a `blob:` or `data:` address).
    ReceiveStart {
        /// A suggested name; the extension is corrected from the bytes.
        name: String,
        #[serde(default)]
        mime: Option<String>,
        #[serde(default)]
        dest_dir: Option<String>,
    },
    /// The next piece (at most 768 KiB before base64), numbered from 0.
    ReceivePiece {
        transfer: u64,
        index: u64,
        data: String,
    },
    /// The file is complete.
    ReceiveEnd {
        transfer: u64,
    },
    /// Give up; what was received is deleted.
    ReceiveAbort {
        transfer: u64,
    },
    /// Continue a failed or paused download from a fresh link to the same file.
    Refresh {
        id: i64,
        url: String,
        #[serde(default)]
        cookies: Option<String>,
        #[serde(default)]
        referrer: Option<String>,
        #[serde(default)]
        user_agent: Option<String>,
    },
    /// The stream downloads started since the daemon began.
    Jobs,
    /// Every download, oldest first.
    List,
    Pause {
        id: i64,
    },
    Resume {
        id: i64,
    },
    Cancel {
        id: i64,
    },
    Remove {
        id: i64,
        #[serde(default)]
        delete_file: bool,
    },
    /// Stop the daemon (running downloads are paused and resume next time).
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadInfo {
    pub id: i64,
    pub url: String,
    pub filename: Option<String>,
    /// `queued`, `downloading`, `paused`, `completed`, `failed`, ...
    pub status: String,
    pub downloaded: u64,
    pub size: Option<u64>,
    pub error: Option<String>,
    /// Where the finished file is.
    pub path: Option<String>,
    /// The page the link was on, for downloads whose link expired.
    #[serde(default)]
    pub page: Option<String>,
    /// The download failed because the server refused its link (403 or 410),
    /// which usually means the link expired and a fresh one may work.
    #[serde(default)]
    pub expired: bool,
}

/// One connection of a running download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// Its number, from 1.
    pub number: u32,
    /// First byte of its part of the file.
    pub start: u64,
    /// One past its last byte.
    pub end: u64,
    /// Bytes of its part received.
    pub received: u64,
    /// Bytes per second over the last seconds.
    pub speed: u64,
    /// `active`, `waiting` (no bytes lately) or `done`.
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryInfo {
    pub id: i64,
    pub name: String,
    pub extensions: Vec<String>,
    /// As set; `None` means a folder with the category's name.
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueInfo {
    pub id: i64,
    pub name: String,
    pub max_active: u32,
    /// Bytes per second; `None` for no limit.
    pub speed_limit: Option<u64>,
    pub windows: Vec<Window>,
    /// Whether its schedule lets it run right now.
    pub open_now: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: u64,
    pub url: String,
    /// `running`, `finished` or `failed`.
    pub status: String,
    /// For a failure: the last thing the program said.
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Receiving {
        transfer: u64,
    },
    Saved {
        path: String,
        name: String,
    },
    Started {
        job: u64,
        /// Where the file will be saved.
        folder: String,
    },
    Jobs {
        jobs: Vec<JobInfo>,
    },
    Pong {
        version: String,
        pid: u32,
    },
    Added {
        id: i64,
    },
    /// Not added because of a duplicate (`duplicates: "skip"`).
    Skipped {
        reason: String,
    },
    Downloads {
        downloads: Vec<DownloadInfo>,
    },
    Details {
        id: i64,
        status: String,
        downloaded: u64,
        size: Option<u64>,
        /// Bytes per second, all connections together.
        speed: u64,
        /// Empty for a download on a single connection.
        connections: Vec<ConnectionInfo>,
    },
    Config {
        /// The folder downloads go to unless told otherwise.
        download_folder: String,
        /// Where it is when the person has not chosen one.
        default_folder: String,
        /// Bytes per second; `None` when there is no limit.
        speed_limit: Option<u64>,
        max_active: u32,
        notifications: bool,
        keep_in_tray: bool,
        sort_by_type: bool,
    },
    Categories {
        categories: Vec<CategoryInfo>,
    },
    Queues {
        queues: Vec<QueueInfo>,
    },
    Plan {
        windows: Vec<Window>,
        /// Whether the schedule lets downloads run right now.
        open_now: bool,
        /// The armed action, if any.
        action: Option<String>,
        /// Seconds until the action happens, while a countdown runs.
        countdown: Option<u32>,
        /// What happened last with the action (done or failed), for the person to read.
        message: Option<String>,
    },
    /// The value of a setting, if there is one.
    Setting {
        value: Option<String>,
    },
    Done,
    Error {
        message: String,
    },
}
