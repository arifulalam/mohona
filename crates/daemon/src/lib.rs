//! The Mohona background engine and its local API.
//!
//! One process keeps the download manager running; the command line, the
//! browser extension (through its native host) and the app are clients of it.
//! The API is one JSON message per line over a TCP connection to `127.0.0.1`,
//! and every message carries a secret token that only the person's own
//! account can read (see [`DaemonInfo`]).

pub mod api;
mod client;
mod info;
mod protocol;
pub mod receive;
mod server;
mod streams;

pub use client::{Client, ClientError};
pub use info::{default_db_path, default_download_dir, default_info_path, DaemonInfo};
pub use protocol::{
    Call, CategoryInfo, ConnectionInfo, DownloadInfo, JobInfo, QueueInfo, Request, Response,
};
pub use server::{serve, ServerConfig};
