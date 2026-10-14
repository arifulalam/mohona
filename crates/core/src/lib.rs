//! Mohona core engine.
//!
//! Phase 1 adds the segmented, resumable download engine here. For now this
//! crate only exposes the version, so the other crates have something to link.

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
