//! Typed access to the downloads and queues tables.

use rusqlite::{params, OptionalExtension, Row};

use crate::{Store, StoreError};

/// Where a download is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Waiting for a free slot in its queue.
    Queued,
    /// Asking the server about the file.
    Probing,
    /// Transferring data.
    Downloading,
    /// Stopped by the user; can be resumed.
    Paused,
    /// Waiting to try again after an error.
    Retrying,
    /// Done; the file is in place.
    Completed,
    /// Gave up after an error.
    Failed,
    /// Stopped by the user for good; partial data was removed.
    Canceled,
}

impl Status {
    /// The text stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::Probing => "probing",
            Status::Downloading => "downloading",
            Status::Paused => "paused",
            Status::Retrying => "retrying",
            Status::Completed => "completed",
            Status::Failed => "failed",
            Status::Canceled => "canceled",
        }
    }

    /// Parse the stored text.
    pub fn parse(text: &str) -> Option<Status> {
        Some(match text {
            "queued" => Status::Queued,
            "probing" => Status::Probing,
            "downloading" => Status::Downloading,
            "paused" => Status::Paused,
            "retrying" => Status::Retrying,
            "completed" => Status::Completed,
            "failed" => Status::Failed,
            "canceled" => Status::Canceled,
            _ => return None,
        })
    }

    /// Whether a download in this state occupies a slot in its queue.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Status::Probing | Status::Downloading | Status::Retrying
        )
    }
}

/// One row of the downloads table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadRow {
    /// Database id.
    pub id: i64,
    /// The address as the user gave it.
    pub url: String,
    /// The address after redirects, once known.
    pub final_url: Option<String>,
    /// File name, once known.
    pub filename: Option<String>,
    /// Folder the file goes in.
    pub dest_dir: String,
    /// Size in bytes, if known.
    pub size: Option<u64>,
    /// Bytes downloaded so far (as last saved).
    pub downloaded: u64,
    /// Current state.
    pub status: Status,
    /// Last error message, if it failed.
    pub error: Option<String>,
    /// Where the finished file was saved.
    pub path: Option<String>,
    /// The queue it belongs to.
    pub queue_id: i64,
    /// Order within the queue (lower runs first).
    pub position: i64,
    /// Unix time it was added.
    pub created_at: i64,
    /// Unix time it completed.
    pub finished_at: Option<i64>,
}

/// One row of the queues table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueRow {
    /// Database id.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// How many downloads may run at once.
    pub max_active: u32,
}

const COLUMNS: &str = "id,url,final_url,filename,dest_dir,size,downloaded,status,error,path,\
                       queue_id,position,created_at,finished_at";

fn row(r: &Row<'_>) -> rusqlite::Result<DownloadRow> {
    let status: String = r.get(7)?;
    Ok(DownloadRow {
        id: r.get(0)?,
        url: r.get(1)?,
        final_url: r.get(2)?,
        filename: r.get(3)?,
        dest_dir: r.get(4)?,
        size: r.get::<_, Option<i64>>(5)?.map(|n| n as u64),
        downloaded: r.get::<_, i64>(6)? as u64,
        status: Status::parse(&status).unwrap_or(Status::Failed),
        error: r.get(8)?,
        path: r.get(9)?,
        queue_id: r.get(10)?,
        position: r.get(11)?,
        created_at: r.get(12)?,
        finished_at: r.get(13)?,
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Store {
    /// Add a download to the end of a queue (the first queue if `queue_id`
    /// is `None`). Returns its id.
    pub fn add_download(
        &self,
        url: &str,
        dest_dir: &str,
        queue_id: Option<i64>,
    ) -> Result<i64, StoreError> {
        let queue_id = match queue_id {
            Some(id) => id,
            None => self
                .conn
                .query_row("SELECT MIN(id) FROM queues", [], |r| r.get(0))?,
        };
        let position: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position),0)+1 FROM downloads WHERE queue_id=?1",
            [queue_id],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO downloads (url,dest_dir,queue_id,position,created_at) \
             VALUES (?1,?2,?3,?4,?5)",
            params![url, dest_dir, queue_id, position, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// One download by id.
    pub fn download(&self, id: i64) -> Result<Option<DownloadRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM downloads WHERE id=?1"),
                [id],
                row,
            )
            .optional()?)
    }

    /// All downloads, oldest first.
    pub fn list_downloads(&self) -> Result<Vec<DownloadRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM downloads ORDER BY id"))?;
        let rows = stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Change a download's state. `error` is stored for `Failed` and cleared
    /// otherwise; `finished_at` is set for `Completed`.
    pub fn set_status(
        &self,
        id: i64,
        status: Status,
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        let finished = (status == Status::Completed).then(now);
        self.conn.execute(
            "UPDATE downloads SET status=?2, error=?3, finished_at=COALESCE(?4, finished_at) \
             WHERE id=?1",
            params![id, status.as_str(), error, finished],
        )?;
        Ok(())
    }

    /// Record what the probe learned.
    pub fn set_started(
        &self,
        id: i64,
        final_url: &str,
        filename: &str,
        size: Option<u64>,
        downloaded: u64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE downloads SET final_url=?2, filename=?3, size=?4, downloaded=?5 WHERE id=?1",
            params![
                id,
                final_url,
                filename,
                size.map(|n| n as i64),
                downloaded as i64
            ],
        )?;
        Ok(())
    }

    /// Save how many bytes are downloaded.
    pub fn set_downloaded(&self, id: i64, downloaded: u64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE downloads SET downloaded=?2 WHERE id=?1",
            params![id, downloaded as i64],
        )?;
        Ok(())
    }

    /// Mark a download complete.
    pub fn set_completed(&self, id: i64, path: &str, bytes: u64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE downloads SET path=?2, size=?3, downloaded=?3 WHERE id=?1",
            params![id, path, bytes as i64],
        )?;
        self.set_status(id, Status::Completed, None)
    }

    /// Remove a download (and, by cascade, its segments and log).
    pub fn delete_download(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM downloads WHERE id=?1", [id])?;
        Ok(())
    }

    /// After a crash or kill, downloads can be left marked as running.
    /// Put them back in the queue; their progress files let them resume.
    /// Returns how many were reset.
    pub fn reset_interrupted(&self) -> Result<usize, StoreError> {
        Ok(self.conn.execute(
            "UPDATE downloads SET status='queued' \
             WHERE status IN ('probing','downloading','retrying')",
            [],
        )?)
    }

    /// All queues.
    pub fn queues(&self) -> Result<Vec<QueueRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,name,max_active FROM queues ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(QueueRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    max_active: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Change how many downloads a queue may run at once (at least 1).
    pub fn set_queue_max_active(&self, queue_id: i64, max_active: u32) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE queues SET max_active=?2 WHERE id=?1",
            params![queue_id, max_active.max(1)],
        )?;
        Ok(())
    }

    /// The queued downloads that should start now: in each queue, the first
    /// ones by position, as many as there are free slots.
    pub fn next_to_start(&self) -> Result<Vec<i64>, StoreError> {
        let mut ids = Vec::new();
        for queue in self.queues()? {
            let active: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM downloads WHERE queue_id=?1 \
                 AND status IN ('probing','downloading','retrying')",
                [queue.id],
                |r| r.get(0),
            )?;
            let free = (queue.max_active as i64 - active).max(0);
            let mut stmt = self.conn.prepare(
                "SELECT id FROM downloads WHERE queue_id=?1 AND status='queued' \
                 ORDER BY position,id LIMIT ?2",
            )?;
            let found = stmt
                .query_map(params![queue.id, free], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            ids.extend(found);
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn added_download_is_queued_at_the_end() {
        let s = store();
        let a = s.add_download("http://a/1", "d", None).unwrap();
        let b = s.add_download("http://a/2", "d", None).unwrap();
        let (ra, rb) = (
            s.download(a).unwrap().unwrap(),
            s.download(b).unwrap().unwrap(),
        );
        assert_eq!(ra.status, Status::Queued);
        assert!(rb.position > ra.position);
        assert_eq!(s.list_downloads().unwrap().len(), 2);
    }

    #[test]
    fn status_text_round_trips() {
        for st in [
            Status::Queued,
            Status::Probing,
            Status::Downloading,
            Status::Paused,
            Status::Retrying,
            Status::Completed,
            Status::Failed,
            Status::Canceled,
        ] {
            assert_eq!(Status::parse(st.as_str()), Some(st));
        }
        assert_eq!(Status::parse("nope"), None);
    }

    #[test]
    fn queue_never_starts_more_than_its_limit() {
        let s = store();
        s.set_queue_max_active(1, 3).unwrap();
        let ids: Vec<i64> = (0..5)
            .map(|i| s.add_download(&format!("http://a/{i}"), "d", None).unwrap())
            .collect();
        let first = s.next_to_start().unwrap();
        assert_eq!(first, ids[..3]);
        s.set_status(ids[0], Status::Downloading, None).unwrap();
        s.set_status(ids[1], Status::Probing, None).unwrap();
        s.set_status(ids[2], Status::Retrying, None).unwrap();
        assert!(s.next_to_start().unwrap().is_empty());
        s.set_status(ids[1], Status::Completed, None).unwrap();
        assert_eq!(s.next_to_start().unwrap(), vec![ids[3]]);
        s.set_status(ids[0], Status::Paused, None).unwrap();
        assert_eq!(s.next_to_start().unwrap(), vec![ids[3], ids[4]]);
    }

    #[test]
    fn interrupted_downloads_go_back_to_the_queue() {
        let s = store();
        let a = s.add_download("u", "d", None).unwrap();
        let b = s.add_download("v", "d", None).unwrap();
        s.set_status(a, Status::Downloading, None).unwrap();
        s.set_status(b, Status::Paused, None).unwrap();
        assert_eq!(s.reset_interrupted().unwrap(), 1);
        assert_eq!(s.download(a).unwrap().unwrap().status, Status::Queued);
        assert_eq!(s.download(b).unwrap().unwrap().status, Status::Paused);
    }

    #[test]
    fn completing_records_path_size_and_time() {
        let s = store();
        let a = s.add_download("u", "d", None).unwrap();
        s.set_started(a, "u2", "f.bin", Some(10), 0).unwrap();
        s.set_completed(a, "d/f.bin", 10).unwrap();
        let r = s.download(a).unwrap().unwrap();
        assert_eq!(r.status, Status::Completed);
        assert_eq!(r.path.as_deref(), Some("d/f.bin"));
        assert_eq!((r.size, r.downloaded), (Some(10), 10));
        assert!(r.finished_at.is_some());
    }

    #[test]
    fn failure_keeps_the_message_and_retry_clears_it() {
        let s = store();
        let a = s.add_download("u", "d", None).unwrap();
        s.set_status(a, Status::Failed, Some("boom")).unwrap();
        assert_eq!(
            s.download(a).unwrap().unwrap().error.as_deref(),
            Some("boom")
        );
        s.set_status(a, Status::Queued, None).unwrap();
        assert_eq!(s.download(a).unwrap().unwrap().error, None);
    }

    #[test]
    fn database_from_schema_1_upgrades_without_losing_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        {
            let c = rusqlite::Connection::open(&path).unwrap();
            c.execute_batch(include_str!("migrations/001_initial.sql"))
                .unwrap();
            c.execute(
                "INSERT INTO downloads (url,dest_dir,queue_id,created_at) VALUES ('u','d',1,5)",
                [],
            )
            .unwrap();
            c.pragma_update(None, "user_version", 1).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), crate::SCHEMA_VERSION);
        let r = s.list_downloads().unwrap();
        assert_eq!((r.len(), r[0].downloaded), (1, 0));
    }
}
