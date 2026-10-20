//! Mohona transports.
//!
//! HTTP(S) lives here now: [`HttpClient`] can [`probe`](HttpClient::probe) a
//! URL and start a download. FTP, SFTP, WebDAV, proxies and authentication
//! follow in Phase 3, all behind one transport interface.

mod auth;
mod checksum;
mod context;
mod disposition;
mod error;
mod ftp;
mod http;

pub use auth::{split_userinfo, Credentials};
pub use checksum::{hash_file, Algorithm, Checksum};
pub use context::{ProxyChoice, RequestContext, TlsOptions};
pub use disposition::filename_from_content_disposition;
pub use error::{Failure, NetError};
pub use http::{Body, ClientConfig, HttpClient, Probe};

/// Names of the transports Mohona plans to support, in build order.
pub const PLANNED_TRANSPORTS: &[&str] = &["http", "https", "ftp", "sftp", "webdav"];

#[cfg(test)]
mod tests {
    use super::PLANNED_TRANSPORTS;

    #[test]
    fn http_comes_first() {
        assert_eq!(PLANNED_TRANSPORTS.first(), Some(&"http"));
    }
}
