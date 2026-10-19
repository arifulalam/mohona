//! Mohona core engine.
//!
//! [`download`] fetches one URL. When the server supports byte ranges, the
//! file is split over several connections that are re-balanced as they go;
//! otherwise it is fetched as a single stream. Resume and retries build on
//! this in the rest of Phase 1.

mod cancel;
mod download;
mod error;
mod filename;
mod ledger;
mod limiter;
mod parallel;
mod retry;
mod segments;
mod tuner;
mod verify;

pub use cancel::CancelToken;
pub use download::{
    discard_partial, download, partial_progress, DownloadRequest, Event, Outcome,
    DEFAULT_CONNECTIONS, MAX_CONNECTIONS,
};
pub use error::Error;
pub use filename::{sanitize_filename, unique_path};
pub use ledger::{Ledger as Progress, ProgressStore};
pub use limiter::RateLimit;
pub use retry::RetryPolicy;
pub use segments::{split_evenly, Claim, Scheduler};
pub use verify::{Source as ChecksumSource, Verified};

/// The version of Mohona, taken from the crate manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_is_set() {
        assert!(!VERSION.is_empty());
    }
}
