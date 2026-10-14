//! Mohona transports.
//!
//! HTTP(S) arrives in Phase 1. FTP, SFTP, WebDAV, proxies and authentication
//! follow in Phase 3, all behind one transport interface.

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
