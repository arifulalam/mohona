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
    /// The rules ("when a download matches this, do that"), in the order they are tried.
    GetRules,
    /// Replace the whole list of rules (checked first; new rules get ids).
    SetRules {
        rules: Vec<crate::rules::Rule>,
    },
    /// Which rule would apply to this address (nothing is added).
    TestRules {
        url: String,
        #[serde(default)]
        referrer: Option<String>,
    },
    /// How to talk to particular sites (connections, speed, headers).
    GetProfiles,
    /// Replace the whole list of site profiles (checked first; new ones get ids).
    SetProfiles {
        profiles: Vec<crate::profiles::Profile>,
    },
    /// The remote companion's port (0 = off) and the port it listens on now.
    GetRemote,
    /// Choose the companion's port (0 = off). Takes effect when the daemon restarts.
    SetRemote {
        port: u16,
    },
    /// A new token for the companion (scope `read` or `write`), shown once.
    MakeToken {
        name: String,
        scope: String,
    },
    /// Merge settings with the file in a synced folder (the chosen one when `folder` is empty).
    SyncNow {
        #[serde(default)]
        folder: Option<String>,
    },
    /// Choose the folder that is synced every few minutes (none: stop).
    SetSync {
        folder: Option<String>,
    },
    GetSync,
    /// The outside programs Mohona can use (ffmpeg, yt-dlp): found or not, and where to get them.
    GetTools,
    /// Choose where a tool is (`name`: `ffmpeg` or `yt-dlp`); no path goes back to searching by itself.
    SetTool {
        name: String,
        path: Option<String>,
    },
    /// Which GitHub repository is asked for new versions (answered as `Update`, nothing checked).
    GetUpdate,
    /// Choose that repository (`owner/name`; empty turns the check off).
    SetUpdateRepo {
        repo: String,
    },
    /// Ask GitHub whether a newer version exists (answered as `Update`).
    CheckUpdate,
    /// How finished downloads are scanned for viruses (answered as `Scan`).
    GetScan,
    /// Choose it: `mode` is `off`, `defender` (Windows Defender) or `custom` (the program and
    /// arguments in `command`, with `{file}` for the file).
    SetScan {
        mode: String,
        #[serde(default)]
        command: String,
    },
    /// The browser whose sign-in yt-dlp uses for video pages (answered as `Setting`).
    GetVideoBrowser,
    /// Choose that browser (`chrome`, `edge`, `firefox`, `brave`, `opera`, `vivaldi`, `chromium`,
    /// `safari`); none means no browser's sign-in is used.
    SetVideoBrowser {
        browser: Option<String>,
    },
    /// How long finished torrents are shared (answered as `Seeding`).
    GetSeeding,
    /// Share finished torrents until this upload ratio is reached and/or this many minutes have
    /// passed. Both empty (or 0): finished torrents are not shared.
    SetSeeding {
        /// In hundredths (150 is a ratio of 1.5).
        ratio: Option<u32>,
        minutes: Option<u64>,
    },
    /// The speed limits of torrents (answered as `TorrentSpeed`).
    GetTorrentSpeed,
    /// The finished torrents that are being shared now (answered as `Sharing`).
    GetSharing,
    /// The kind of file taken from video pages (answered as `VideoFormat`).
    GetVideoFormat,
    /// Choose it: quality (`best`, `2160` … `360`, `audio`), video format and audio format.
    SetVideoFormat {
        quality: String,
        container: String,
        audio: String,
    },
    /// Stop sharing one of them (answered as `Sharing`, the list that is left).
    StopSharing {
        id: u64,
    },
    /// Limit all torrents together, in KB/s (empty or 0: no limit).
    SetTorrentSpeed {
        down_kb: Option<u32>,
        up_kb: Option<u32>,
    },
    /// The plugins found in the plugins folder.
    GetPlugins,
    /// Switch a plugin on or off (by its name).
    SetPlugin {
        name: String,
        enabled: bool,
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
        /// A torrent: only these files (numbers from `TorrentFiles`); default: all.
        #[serde(default)]
        files: Option<Vec<u32>>,
        /// The link is a web page whose video is wanted (taken by yt-dlp; no video is an error).
        #[serde(default)]
        video: bool,
        /// For a video page: quality, format and sound format for this download only
        /// (default: the saved settings).
        #[serde(default)]
        choice: Option<mohona_net::VideoChoice>,
    },
    /// Start the site grabber on a web page (one run at a time; follow it with `GrabStatus`).
    GrabStart {
        url: String,
        /// 0: only this page; 1: also the pages it links to; ... (at most 5)
        #[serde(default)]
        depth: u32,
        /// Follow links to pages of the same site only.
        #[serde(default = "yes")]
        same_site: bool,
        /// File groups (`images`, `video`, `audio`, `documents`, `archives`, `programs`) or
        /// endings (`pdf`); none: every file.
        #[serde(default)]
        types: Vec<String>,
        /// Most pages to read (default 100, at most 500).
        #[serde(default)]
        max_pages: Option<u32>,
        /// Obey the site's robots.txt (default yes).
        #[serde(default = "yes")]
        robots: bool,
    },
    /// How the grabber is doing, and what it found (answered as `Grab`).
    GrabStatus,
    /// Stop the grabber; what it found so far stays.
    GrabStop,
    /// What is inside a torrent (a magnet link or a `.torrent` address); can take up to 90 seconds.
    TorrentFiles {
        url: String,
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

fn yes() -> bool {
    true
}

/// One file the site grabber found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrabFile {
    pub url: String,
    pub name: String,
    pub group: String,
    /// The page that links to it.
    pub page: String,
}

/// One file inside a torrent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentFileInfo {
    pub index: u32,
    pub path: String,
    pub size: u64,
}

/// One outside program, as the person sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStatus {
    /// `ffmpeg` or `yt-dlp`.
    pub name: String,
    /// What it is used for ("... needed to ...").
    pub purpose: String,
    /// The path the person chose, if any.
    pub chosen: Option<String>,
    /// The file that will be used, if one is found.
    pub found: Option<String>,
    pub page: String,
    pub windows_page: String,
    pub winget: String,
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
        /// The earlier download, when the address is already in the list.
        #[serde(default)]
        id: Option<i64>,
        /// Its state (`completed`, `downloading`, ...).
        #[serde(default)]
        status: Option<String>,
        /// Where its file is (or was).
        #[serde(default)]
        path: Option<String>,
        /// The file is still there.
        #[serde(default)]
        exists: bool,
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
    Rules {
        rules: Vec<crate::rules::Rule>,
    },
    Profiles {
        profiles: Vec<crate::profiles::Profile>,
    },
    TorrentFiles {
        name: String,
        files: Vec<TorrentFileInfo>,
    },
    /// Where new versions are looked for, and what was found (`latest` is empty until checked).
    Update {
        repo: String,
        current: String,
        latest: Option<String>,
        newer: bool,
        url: Option<String>,
        notes: Option<String>,
    },
    /// The virus scan setting.
    Scan {
        mode: String,
        command: String,
        /// Where Windows Defender's command line tool is, if this computer has it.
        defender: Option<String>,
    },
    /// The site grabber: `idle` (never run), `running`, `done`, `stopped` or `failed`.
    Grab {
        state: String,
        pages: u32,
        found: u32,
        /// The page being read.
        current: String,
        /// The files found (listed once the run is over, so a status call stays small).
        files: Vec<GrabFile>,
        skipped_by_robots: u32,
        failed_pages: u32,
        note: Option<String>,
        error: Option<String>,
    },
    Tools {
        tools: Vec<ToolStatus>,
    },
    Plugins {
        /// Where plugins are kept.
        folder: String,
        plugins: Vec<crate::plugins::PluginInfo>,
    },
    Synced {
        /// Settings changed on this computer.
        applied: Vec<String>,
        /// Settings written to the file.
        wrote: Vec<String>,
    },
    SyncFolder {
        folder: Option<String>,
    },
    Remote {
        /// Chosen port, 0 for off.
        port: u16,
        /// The port it listens on now, 0 for not listening.
        listening: u16,
    },
    Token {
        id: i64,
        name: String,
        scope: String,
        token: String,
    },
    /// The rule that applies, if any.
    RuleMatch {
        rule: Option<crate::rules::Rule>,
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
    /// How finished torrents are shared (none set: they are not).
    Seeding {
        /// In hundredths (150 is a ratio of 1.5).
        ratio: Option<u32>,
        minutes: Option<u64>,
    },
    /// The kind of file taken from video pages, and the choices there are.
    VideoFormat {
        quality: String,
        container: String,
        audio: String,
    },
    /// The torrents being shared now.
    Sharing {
        items: Vec<SharedItem>,
    },
    /// The speed limits of torrents, in KB/s (none: no limit).
    TorrentSpeed {
        down_kb: Option<u32>,
        up_kb: Option<u32>,
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

/// A finished torrent that is being shared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedItem {
    pub id: u64,
    pub name: String,
    /// The size of the torrent, and how much of it others have taken so far.
    pub total: u64,
    pub uploaded: u64,
    /// For how long it has been shared.
    pub seconds: u64,
}
