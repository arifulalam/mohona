//! The Mohona background engine and its local API.
//!
//! One process keeps the download manager running; the command line, the
//! browser extension (through its native host) and the app are clients of it.
//! The API is one JSON message per line over a TCP connection to `127.0.0.1`,
//! and every message carries a secret token that only the person's own
//! account can read (see [`DaemonInfo`]).

mod client;
mod info;
mod protocol;
mod server;

pub use client::{Client, ClientError};
pub use info::{default_db_path, default_info_path, DaemonInfo};
pub use protocol::{Call, DownloadInfo, Request, Response};
pub use server::{serve, ServerConfig};
