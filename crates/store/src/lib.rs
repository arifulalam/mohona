//! Mohona storage: the SQLite database (WAL mode) with versioned migrations.

use rusqlite::Connection;
use std::path::Path;

/// One migration: the version it brings the database to, and its SQL.
struct Migration {
    version: u32,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("migrations/001_initial.sql"),
}];

/// Version of the database schema after all migrations have run.
pub const SCHEMA_VERSION: u32 = 1;

/// Errors from the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite reported an error.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The database was written by a newer Mohona and must not be touched.
    #[error(
        "the database is from a newer version of Mohona (schema {found}, this build knows {known})"
    )]
    TooNew {
        /// Version found in the file.
        found: u32,
        /// Newest version this build understands.
        known: u32,
    },
}

/// An open Mohona database.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the database at `path` and bring it up to date.
    pub fn open(path: &Path) -> Result<Store, StoreError> {
        Self::init(Connection::open(path)?)
    }

    /// An in-memory database, for tests.
    pub fn open_in_memory() -> Result<Store, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store, StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    /// The schema version stored in the file.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// The underlying connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Each migration runs in its own transaction, so a failure leaves the
    /// database at the previous version rather than half-upgraded.
    fn migrate(&mut self) -> Result<(), StoreError> {
        let current = self.schema_version()?;
        if current > SCHEMA_VERSION {
            return Err(StoreError::TooNew {
                found: current,
                known: SCHEMA_VERSION,
            });
        }
        for m in MIGRATIONS.iter().filter(|m| m.version > current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(m.sql)?;
            tx.pragma_update(None, "user_version", m.version)?;
            tx.commit()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_numbered_in_order() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1);
        }
        assert_eq!(MIGRATIONS.last().unwrap().version, SCHEMA_VERSION);
    }

    #[test]
    fn new_database_has_default_queue_and_categories() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        let q: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM queues", [], |r| r.get(0))
            .unwrap();
        let c: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert_eq!((q, c), (1, 5));
    }

    #[test]
    fn reopening_keeps_data_and_does_not_rerun_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        {
            let s = Store::open(&path).unwrap();
            s.conn()
                .execute("INSERT INTO settings VALUES ('theme','dark')", [])
                .unwrap();
        }
        let s = Store::open(&path).unwrap();
        let v: String = s
            .conn()
            .query_row("SELECT value FROM settings WHERE key='theme'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(v, "dark");
        let q: i64 = s
            .conn()
            .query_row("SELECT COUNT(*) FROM queues", [], |r| r.get(0))
            .unwrap();
        assert_eq!(q, 1);
    }

    #[test]
    fn refuses_a_database_from_the_future() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        Store::open(&path).unwrap();
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(matches!(Store::open(&path), Err(StoreError::TooNew { .. })));
    }

    #[test]
    fn deleting_a_download_removes_its_segments_and_log() {
        let s = Store::open_in_memory().unwrap();
        let c = s.conn();
        c.execute(
            "INSERT INTO downloads (url,dest_dir,queue_id,created_at) VALUES ('u','d',1,0)",
            [],
        )
        .unwrap();
        c.execute("INSERT INTO segments VALUES (1,0,10,100)", [])
            .unwrap();
        c.execute(
            "INSERT INTO log (download_id,at,level,message) VALUES (1,0,'info','x')",
            [],
        )
        .unwrap();
        c.execute("DELETE FROM downloads WHERE id=1", []).unwrap();
        let n: i64 = c
            .query_row(
                "SELECT (SELECT COUNT(*) FROM segments)+(SELECT COUNT(*) FROM log)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn bad_status_is_rejected() {
        let s = Store::open_in_memory().unwrap();
        let r = s.conn().execute(
            "INSERT INTO downloads (url,dest_dir,queue_id,created_at,status) VALUES ('u','d',1,0,'weird')",
            [],
        );
        assert!(r.is_err());
    }
}
