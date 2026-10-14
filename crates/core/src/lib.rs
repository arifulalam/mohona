//! Mohona core engine.
//!
//! Today it downloads one URL as a single stream ([`download`]). The
//! segmented, resumable engine builds on this in the rest of Phase 1.

mod download;
mod error;
mod filename;

pub use download::{download, DownloadRequest, Event, Outcome};
pub use error::Error;
pub use filename::{sanitize_filename, unique_path};

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
