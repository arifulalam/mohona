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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
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
    Done,
    Error {
        message: String,
    },
}
