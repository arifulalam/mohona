//! Mohona storage.
//!
//! Phase 2 adds the SQLite schema (downloads, segments, queues, categories,
//! settings, log) with versioned migrations.

/// Version of the database schema. Phase 2 starts it at 1.
pub const SCHEMA_VERSION: u32 = 0;

#[cfg(test)]
mod tests {
    use super::SCHEMA_VERSION;

    #[test]
    fn schema_has_not_started_yet() {
        assert_eq!(SCHEMA_VERSION, 0);
    }
}
