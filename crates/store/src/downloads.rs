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
    /// Its own speed limit in bytes per second, if any.
    pub speed_limit: Option<u64>,
    /// The category it was sorted into when it finished, if any.
    pub category_id: Option<i64>,
    /// Whether the finished file is sorted into its category's folder.
    pub auto_category: bool,
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
    /// The queue's speed limit in bytes per second, if any.
    pub speed_limit: Option<u64>,
    /// The queue's schedule as JSON, if it has one (see `mohona-manager`).
    pub schedule_json: Option<String>,
}

/// What is saved about a download in progress (see `Store::save_progress`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressRecord {
    /// The address after redirects.
    pub final_url: String,
    /// The file name the result gets.
    pub filename: String,
    /// Size of the whole file.
    pub total: u64,
    /// `ETag` the server sent, if any.
    pub etag: Option<String>,
    /// `Last-Modified` the server sent, if any.
    pub last_modified: Option<String>,
    /// `(start, committed, end)` of each segment.
    pub segments: Vec<(u64, u64, u64)>,
}

pub(crate) const COLUMNS: &str = "id,url,final_url,filename,dest_dir,size,downloaded,status,error,path,\
                       queue_id,position,created_at,finished_at,speed_limit,category_id,auto_category";

pub(crate) fn row(r: &Row<'_>) -> rusqlite::Result<DownloadRow> {
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
        speed_limit: r.get::<_, Option<i64>>(14)?.map(|n| n as u64),
        category_id: r.get(15)?,
        auto_category: r.get::<_, i64>(16)? != 0,
    })
}

/// An address without surrounding spaces and without a `#fragment`, which
/// the server never sees.
fn same_address(url: &str) -> &str {
    let url = url.trim();
    url.split_once('#').map_or(url, |(address, _)| address)
}

pub(crate) fn now() -> i64 {
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
        self.add_download_with(url, dest_dir, queue_id, false)
    }

    /// Like [`Store::add_download`]. With `auto_category`, the finished file
    /// is sorted into its category's folder.
    pub fn add_download_with(
        &self,
        url: &str,
        dest_dir: &str,
        queue_id: Option<i64>,
        auto_category: bool,
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
            "INSERT INTO downloads (url,dest_dir,queue_id,position,created_at,auto_category) \
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                url,
                dest_dir,
                queue_id,
                position,
                now(),
                auto_category as i64
            ],
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

    /// Record the category a finished download was sorted into.
    pub fn set_category(&self, id: i64, category_id: Option<i64>) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE downloads SET category_id=?2 WHERE id=?1",
            params![id, category_id],
        )?;
        Ok(())
    }

    /// Downloads of the same address (ignoring a `#fragment` and spaces
    /// around it), oldest first.
    pub fn downloads_with_url(&self, url: &str) -> Result<Vec<DownloadRow>, StoreError> {
        let wanted = same_address(url);
        Ok(self
            .list_downloads()?
            .into_iter()
            .filter(|row| same_address(&row.url) == wanted)
            .collect())
    }

    /// The request details (cookies, headers) saved with a download, as the
    /// JSON `mohona-net` writes, if any.
    pub fn headers_json(&self, id: i64) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT headers_json FROM downloads WHERE id=?1",
                [id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Point a download at a new address (a fresh link to the same file).
    pub fn set_url(&self, id: i64, url: &str) -> Result<(), StoreError> {
        self.conn
            .execute("UPDATE downloads SET url=?2 WHERE id=?1", params![id, url])?;
        Ok(())
    }

    /// Save request details (cookies, headers) with a download.
    pub fn set_headers_json(&self, id: i64, json: Option<&str>) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE downloads SET headers_json=?2 WHERE id=?1",
            params![id, json],
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

    /// Save a download's progress: what the server said about the file and
    /// how far each segment is. It is all-or-nothing (one transaction), so
    /// after a crash the previous snapshot or the new one is there.
    pub fn save_progress(&self, id: i64, record: &ProgressRecord) -> Result<(), StoreError> {
        // IMMEDIATE takes the write lock at the start, so the busy timeout
        // applies. A plain (deferred) transaction that has to upgrade from
        // reading to writing fails at once when another connection wrote
        // in between, and with many downloads saving progress that happened
        // all the time.
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let committed: u64 = record.segments.iter().map(|(s, c, _)| c - s).sum();
        tx.execute(
            "UPDATE downloads SET final_url=?2, filename=?3, size=?4, etag=?5, \
             last_modified=?6, downloaded=?7 WHERE id=?1",
            params![
                id,
                record.final_url,
                record.filename,
                record.total as i64,
                record.etag,
                record.last_modified,
                committed as i64
            ],
        )?;
        tx.execute("DELETE FROM segments WHERE download_id=?1", [id])?;
        let mut insert = tx.prepare(
            "INSERT INTO segments (download_id,start,committed,\"end\") VALUES (?1,?2,?3,?4)",
        )?;
        for (start, done, end) in &record.segments {
            insert.execute(params![id, *start as i64, *done as i64, *end as i64])?;
        }
        drop(insert);
        tx.commit()?;
        Ok(())
    }

    /// The saved progress of a download, if it has any segments.
    pub fn load_progress(&self, id: i64) -> Result<Option<ProgressRecord>, StoreError> {
        let head = self
            .conn
            .query_row(
                "SELECT final_url,filename,size,etag,last_modified FROM downloads WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((Some(final_url), Some(filename), Some(total), etag, last_modified)) = head else {
            return Ok(None);
        };
        let mut stmt = self.conn.prepare(
            "SELECT start,committed,\"end\" FROM segments WHERE download_id=?1 ORDER BY start",
        )?;
        let segments = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, i64>(2)? as u64,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if segments.is_empty() {
            return Ok(None);
        }
        Ok(Some(ProgressRecord {
            final_url,
            filename,
            total: total as u64,
            etag,
            last_modified,
            segments,
        }))
    }

    /// Forget a download's segments.
    pub fn clear_progress(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM segments WHERE download_id=?1", [id])?;
        Ok(())
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
        let mut stmt = self.conn.prepare(
            "SELECT id,name,max_active,speed_limit,schedule_json FROM queues ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(QueueRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    max_active: r.get(2)?,
                    speed_limit: r.get::<_, Option<i64>>(3)?.map(|n| n as u64),
                    schedule_json: r.get(4)?,
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

    /// Set (or with `None` remove) a queue's speed limit, in bytes per second.
    pub fn set_queue_speed_limit(
        &self,
        queue_id: i64,
        limit: Option<u64>,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE queues SET speed_limit=?2 WHERE id=?1",
            params![queue_id, limit.map(|n| n as i64)],
        )?;
        Ok(())
    }

    /// Set (or with `None` remove) a queue's schedule, stored as JSON.
    pub fn set_queue_schedule(
        &self,
        queue_id: i64,
        schedule_json: Option<&str>,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE queues SET schedule_json=?2 WHERE id=?1",
            params![queue_id, schedule_json],
        )?;
        Ok(())
    }

    /// Set (or with `None` remove) one download's speed limit.
    pub fn set_download_speed_limit(&self, id: i64, limit: Option<u64>) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE downloads SET speed_limit=?2 WHERE id=?1",
            params![id, limit.map(|n| n as i64)],
        )?;
        Ok(())
    }

    /// A value from the settings table.
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Save a value in the settings table.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings (key,value) VALUES (?1,?2) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Remove a value from the settings table.
    pub fn remove_setting(&self, key: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM settings WHERE key=?1", [key])?;
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
    fn progress_round_trips_and_replaces_the_earlier_snapshot() {
        let s = store();
        let id = s.add_download("u", "d", None).unwrap();
        assert_eq!(s.load_progress(id).unwrap(), None);
        let mut record = ProgressRecord {
            final_url: "u2".into(),
            filename: "f.bin".into(),
            total: 100,
            etag: Some("\"x\"".into()),
            last_modified: None,
            segments: vec![(0, 30, 50), (50, 50, 100)],
        };
        s.save_progress(id, &record).unwrap();
        assert_eq!(s.load_progress(id).unwrap(), Some(record.clone()));
        assert_eq!(s.download(id).unwrap().unwrap().downloaded, 30);
        record.segments = vec![(0, 50, 100)];
        s.save_progress(id, &record).unwrap();
        assert_eq!(s.load_progress(id).unwrap(), Some(record));
        s.clear_progress(id).unwrap();
        assert_eq!(s.load_progress(id).unwrap(), None);
    }

    #[test]
    fn speed_limits_and_settings_are_saved() {
        let s = store();
        let id = s.add_download("u", "d", None).unwrap();
        assert_eq!(s.download(id).unwrap().unwrap().speed_limit, None);
        s.set_download_speed_limit(id, Some(500)).unwrap();
        assert_eq!(s.download(id).unwrap().unwrap().speed_limit, Some(500));
        s.set_queue_speed_limit(1, Some(9000)).unwrap();
        assert_eq!(s.queues().unwrap()[0].speed_limit, Some(9000));
        s.set_queue_speed_limit(1, None).unwrap();
        assert_eq!(s.queues().unwrap()[0].speed_limit, None);
        assert_eq!(s.queues().unwrap()[0].schedule_json, None);
        s.set_queue_schedule(1, Some("{}")).unwrap();
        assert_eq!(s.queues().unwrap()[0].schedule_json.as_deref(), Some("{}"));
        s.set_queue_schedule(1, None).unwrap();
        assert_eq!(s.queues().unwrap()[0].schedule_json, None);
        assert_eq!(s.setting("speed_limit").unwrap(), None);
        s.set_setting("speed_limit", "1").unwrap();
        s.set_setting("speed_limit", "2").unwrap();
        assert_eq!(s.setting("speed_limit").unwrap().as_deref(), Some("2"));
        s.remove_setting("speed_limit").unwrap();
        assert_eq!(s.setting("speed_limit").unwrap(), None);
    }

    #[test]
    fn the_same_address_is_found_again() {
        let s = store();
        s.add_download("http://a/f.zip#top", "d", None).unwrap();
        s.add_download("http://a/g.zip", "d", None).unwrap();
        assert_eq!(s.downloads_with_url(" http://a/f.zip ").unwrap().len(), 1);
        assert_eq!(s.downloads_with_url("http://a/h.zip").unwrap().len(), 0);
    }

    #[test]
    fn auto_category_is_remembered() {
        let s = store();
        let a = s.add_download_with("u", "d", None, true).unwrap();
        let b = s.add_download("v", "d", None).unwrap();
        assert!(s.download(a).unwrap().unwrap().auto_category);
        assert!(!s.download(b).unwrap().unwrap().auto_category);
        s.set_category(a, Some(2)).unwrap();
        assert_eq!(s.download(a).unwrap().unwrap().category_id, Some(2));
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

    #[test]
    fn many_connections_can_save_progress_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.db");
        let first = Store::open(&path).unwrap();
        let ids: Vec<i64> = (0..8)
            .map(|i| first.add_download(&format!("u{i}"), "d", None).unwrap())
            .collect();
        let threads: Vec<_> = ids
            .into_iter()
            .map(|id| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let store = Store::open(&path).unwrap();
                    for round in 0..50u64 {
                        let record = ProgressRecord {
                            final_url: "u".into(),
                            filename: "f".into(),
                            total: 1000,
                            etag: None,
                            last_modified: None,
                            segments: vec![(0, round, 500), (500, 500 + round, 1000)],
                        };
                        store
                            .save_progress(id, &record)
                            .expect("save must not fail");
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
    }
}
