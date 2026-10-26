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
    Config {
        /// The folder downloads go to unless told otherwise.
        download_folder: String,
        /// Where it is when the person has not chosen one.
        default_folder: String,
        /// Bytes per second; `None` when there is no limit.
        speed_limit: Option<u64>,
        max_active: u32,
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
