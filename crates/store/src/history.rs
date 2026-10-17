//! History: searching the download list, and the per-download log.

use rusqlite::{params_from_iter, types::Value};

use crate::downloads::{row, Status, COLUMNS};
use crate::{DownloadRow, Store, StoreError};

/// What to look for in the history. Everything is optional; all given
/// conditions must hold.
#[derive(Clone, Debug, Default)]
pub struct HistoryQuery {
    /// Words to find in the name, address or category. Every word must
    /// match, and a word matches the start of a word in the text, so `ubu`
    /// finds `ubuntu-24.04.iso`.
    pub text: Option<String>,
    /// Only downloads in this state.
    pub status: Option<Status>,
    /// Only downloads added at or after this unix time.
    pub since: Option<i64>,
    /// Only downloads added before this unix time.
    pub until: Option<i64>,
    /// Only downloads sorted into the category with this name.
    pub category: Option<String>,
    /// At most this many rows (newest first). `None` means all.
    pub limit: Option<u32>,
}

/// One line of a download's log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// Unix time.
    pub at: i64,
    /// `info`, `warn` or `error`.
    pub level: String,
    /// What happened.
    pub message: String,
}

/// Turn what a person typed into an FTS5 query: words made of letters and
/// digits, each matching as a prefix, all required. Everything else is
/// dropped, so no input can be a syntax error. `None` if no words are left.
pub(crate) fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

impl Store {
    /// Search the downloads, newest first.
    pub fn search(&self, query: &HistoryQuery) -> Result<Vec<DownloadRow>, StoreError> {
        let mut sql = format!("SELECT {COLUMNS} FROM downloads WHERE 1=1");
        let mut args: Vec<Value> = Vec::new();
        if let Some(text) = &query.text {
            match fts_query(text) {
                Some(fts) => {
                    sql.push_str(
                        " AND id IN (SELECT rowid FROM downloads_fts WHERE downloads_fts MATCH ?)",
                    );
                    args.push(Value::Text(fts));
                }
                // Only punctuation was typed: nothing can match.
                None if !text.trim().is_empty() => return Ok(Vec::new()),
                None => {}
            }
        }
        if let Some(status) = query.status {
            sql.push_str(" AND status = ?");
            args.push(Value::Text(status.as_str().to_string()));
        }
        if let Some(since) = query.since {
            sql.push_str(" AND created_at >= ?");
            args.push(Value::Integer(since));
        }
        if let Some(until) = query.until {
            sql.push_str(" AND created_at < ?");
            args.push(Value::Integer(until));
        }
        if let Some(category) = &query.category {
            sql.push_str(
                " AND category_id IN (SELECT id FROM categories WHERE name = ? COLLATE NOCASE)",
            );
            args.push(Value::Text(category.clone()));
        }
        sql.push_str(" ORDER BY created_at DESC, id DESC");
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(args), row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Append a line to a download's log.
    pub fn add_log(&self, download_id: i64, level: &str, message: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO log (download_id, at, level, message) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![download_id, crate::downloads::now(), level, message],
        )?;
        Ok(())
    }

    /// A download's log, oldest first.
    pub fn log_of(&self, download_id: i64) -> Result<Vec<LogLine>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT at, level, message FROM log WHERE download_id=?1 ORDER BY id")?;
        let lines = stmt
            .query_map([download_id], |r| {
                Ok(LogLine {
                    at: r.get(0)?,
                    level: r.get(1)?,
                    message: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(s: &Store, url: &str, name: &str, created: i64) -> i64 {
        let id = s.add_download(url, "d", None).unwrap();
        s.conn
            .execute(
                "UPDATE downloads SET created_at=?2 WHERE id=?1",
                rusqlite::params![id, created],
            )
            .unwrap();
        s.set_started(id, url, name, Some(10), 0).unwrap();
        id
    }

    fn ids(rows: Vec<DownloadRow>) -> Vec<i64> {
        rows.into_iter().map(|r| r.id).collect()
    }

    fn text(t: &str) -> HistoryQuery {
        HistoryQuery {
            text: Some(t.into()),
            ..HistoryQuery::default()
        }
    }

    #[test]
    fn fts_query_is_safe_and_prefix_based() {
        assert_eq!(fts_query("ubu 24.04").unwrap(), "\"ubu\"* \"24\"* \"04\"*");
        assert_eq!(fts_query("a\" OR b*").unwrap(), "\"a\"* \"OR\"* \"b\"*");
        assert_eq!(fts_query("  -- ()"), None);
    }

    #[test]
    fn finds_by_name_url_and_prefix() {
        let s = Store::open_in_memory().unwrap();
        let a = add(
            &s,
            "https://example.org/dl/ubuntu-24.04.iso",
            "ubuntu-24.04.iso",
            100,
        );
        let b = add(&s, "https://files.test/report.pdf", "report.pdf", 200);
        assert_eq!(ids(s.search(&text("ubu")).unwrap()), vec![a]);
        assert_eq!(ids(s.search(&text("ISO")).unwrap()), vec![a]);
        assert_eq!(ids(s.search(&text("files.test")).unwrap()), vec![b]);
        assert_eq!(
            ids(s.search(&text("ubuntu report")).unwrap()),
            Vec::<i64>::new()
        );
        assert_eq!(ids(s.search(&text("")).unwrap()), vec![b, a]);
        assert_eq!(ids(s.search(&text("(((")).unwrap()), Vec::<i64>::new());
    }

    #[test]
    fn follows_renames_deletes_and_categories() {
        let s = Store::open_in_memory().unwrap();
        let a = add(&s, "https://x.test/a", "old-name.bin", 1);
        s.set_started(a, "https://x.test/a", "new-name.bin", Some(1), 0)
            .unwrap();
        assert!(s.search(&text("old")).unwrap().is_empty());
        assert_eq!(ids(s.search(&text("new")).unwrap()), vec![a]);

        let c = s
            .add_category("Samples", &["bin".to_string()], None)
            .unwrap();
        s.set_category(a, Some(c)).unwrap();
        assert_eq!(ids(s.search(&text("samples")).unwrap()), vec![a]);

        s.delete_download(a).unwrap();
        assert!(s.search(&text("new")).unwrap().is_empty());
    }

    #[test]
    fn filters_combine() {
        let s = Store::open_in_memory().unwrap();
        let a = add(&s, "https://x.test/a", "a.zip", 100);
        let b = add(&s, "https://x.test/b", "b.zip", 200);
        let c = add(&s, "https://x.test/c", "c.mp4", 300);
        s.set_status(b, Status::Completed, None).unwrap();
        let video = s.category_for("c.mp4").unwrap().unwrap();
        s.set_category(c, Some(video.id)).unwrap();

        let q = |f: &dyn Fn(&mut HistoryQuery)| {
            let mut q = HistoryQuery::default();
            f(&mut q);
            ids(s.search(&q).unwrap())
        };
        assert_eq!(q(&|_| {}), vec![c, b, a]);
        assert_eq!(q(&|q| q.status = Some(Status::Completed)), vec![b]);
        assert_eq!(q(&|q| q.since = Some(200)), vec![c, b]);
        assert_eq!(q(&|q| q.until = Some(200)), vec![a]);
        assert_eq!(q(&|q| q.category = Some("video".into())), vec![c]);
        assert_eq!(q(&|q| q.limit = Some(1)), vec![c]);
        assert_eq!(
            q(&|q| {
                q.text = Some("zip".into());
                q.since = Some(150);
            }),
            vec![b]
        );
    }

    #[test]
    fn upgrade_indexes_existing_rows() {
        let s = Store::open_in_memory().unwrap();
        let a = add(&s, "https://x.test/a", "keep.bin", 1);
        s.conn.execute_batch("DELETE FROM downloads_fts").unwrap();
        assert!(s.search(&text("keep")).unwrap().is_empty());
        // The migration's backfill statement indexes what is there.
        s.conn
            .execute_batch(
                "INSERT INTO downloads_fts (rowid, filename, url, category) \
                 SELECT id, COALESCE(filename,''), url, '' FROM downloads",
            )
            .unwrap();
        assert_eq!(ids(s.search(&text("keep")).unwrap()), vec![a]);
    }

    #[test]
    fn keeps_a_log_per_download() {
        let s = Store::open_in_memory().unwrap();
        let a = add(&s, "https://x.test/a", "a", 1);
        s.add_log(a, "info", "started").unwrap();
        s.add_log(a, "error", "boom").unwrap();
        let lines = s.log_of(a).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].level, "error");
        s.delete_download(a).unwrap();
        assert!(s.log_of(a).unwrap().is_empty());
    }
}
