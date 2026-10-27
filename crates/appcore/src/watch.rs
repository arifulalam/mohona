//! Notices the app shows (a finished download, a failed one). The window may
//! be hidden in the tray, so this runs in the Rust part and only looks at
//! the lists the engine gives.

use std::collections::HashMap;

use serde::Serialize;

use crate::rows::Row;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Compares each list with the one before it.
#[derive(Default)]
pub struct Watcher {
    /// The status of each download at the last look; `None` before the first.
    seen: Option<HashMap<i64, String>>,
}

impl Watcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets what it has seen (when the engine was away): the next list
    /// is taken as the starting point, so nothing old is announced again.
    pub fn reset(&mut self) {
        self.seen = None;
    }

    /// The notices for what changed since the last list. The first list only
    /// sets the starting point.
    pub fn observe(&mut self, rows: &[Row]) -> Vec<Notice> {
        let now: HashMap<i64, String> = rows.iter().map(|r| (r.id, r.status.clone())).collect();
        let Some(before) = self.seen.replace(now) else {
            return Vec::new();
        };
        rows.iter()
            .filter_map(|row| {
                let was = before.get(&row.id).map(String::as_str);
                if was == Some(row.status.as_str()) {
                    return None;
                }
                match row.status.as_str() {
                    "completed" => Some(Notice {
                        title: "Download finished".into(),
                        body: row.name.clone(),
                    }),
                    "failed" => Some(Notice {
                        title: "Download failed".into(),
                        body: format!(
                            "{}: {}",
                            row.name,
                            if row.expired {
                                "the link has expired".to_string()
                            } else {
                                row.error.clone().unwrap_or_else(|| "it stopped".into())
                            }
                        ),
                    }),
                    _ => None,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, status: &str) -> Row {
        Row {
            id,
            name: format!("file{id}.zip"),
            url: String::new(),
            status: status.into(),
            downloaded: 0,
            size: None,
            percent: None,
            speed: 0,
            eta: None,
            error: None,
            path: None,
            expired: false,
        }
    }

    #[test]
    fn the_first_list_announces_nothing() {
        let mut w = Watcher::new();
        assert!(w
            .observe(&[row(1, "completed"), row(2, "failed")])
            .is_empty());
        assert!(w
            .observe(&[row(1, "completed"), row(2, "failed")])
            .is_empty());
    }

    #[test]
    fn a_download_that_finishes_or_fails_is_announced_once() {
        let mut w = Watcher::new();
        w.observe(&[row(1, "downloading"), row(2, "downloading")]);
        let mut failed = row(2, "failed");
        failed.error = Some("server answered 500".into());
        let notices = w.observe(&[row(1, "completed"), failed.clone()]);
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].title, "Download finished");
        assert_eq!(notices[0].body, "file1.zip");
        assert_eq!(notices[1].body, "file2.zip: server answered 500");
        assert!(w.observe(&[row(1, "completed"), failed]).is_empty());
    }

    #[test]
    fn a_new_download_that_is_already_done_is_announced_and_other_changes_are_not() {
        let mut w = Watcher::new();
        w.observe(&[row(1, "downloading")]);
        let quiet = w.observe(&[row(1, "paused"), row(2, "queued")]);
        assert!(quiet.is_empty());
        let notices = w.observe(&[row(1, "paused"), row(2, "completed")]);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].body, "file2.zip");
    }

    #[test]
    fn an_expired_link_is_said_in_words_and_a_reset_forgets_the_past() {
        let mut w = Watcher::new();
        w.observe(&[row(1, "downloading")]);
        let mut r = row(1, "failed");
        r.expired = true;
        r.error = Some("server answered 403".into());
        assert_eq!(
            w.observe(&[r.clone()])[0].body,
            "file1.zip: the link has expired"
        );
        w.reset();
        assert!(w.observe(&[r]).is_empty());
    }
}
