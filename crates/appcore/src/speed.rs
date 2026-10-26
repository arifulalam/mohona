use std::collections::HashMap;
use std::time::Instant;

use mohona_daemon::DownloadInfo;

use crate::rows::{row, Row};

/// Works out each download's speed from how much it grew between two lists.
/// The speed is smoothed so the number on screen does not jump.
#[derive(Default)]
pub struct SpeedTracker {
    last: HashMap<i64, (u64, Instant, f64)>,
}

/// How much of the newest measurement is used (the rest is the earlier speed).
const WEIGHT: f64 = 0.4;

impl SpeedTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rows(&mut self, downloads: Vec<DownloadInfo>, now: Instant) -> Vec<Row> {
        let mut next = HashMap::with_capacity(downloads.len());
        let mut rows = Vec::with_capacity(downloads.len());
        for info in downloads {
            let speed = if info.status == "downloading" {
                match self.last.get(&info.id) {
                    Some(&(before, at, smooth)) if info.downloaded >= before => {
                        let seconds = now.saturating_duration_since(at).as_secs_f64();
                        if seconds >= 0.2 {
                            let measured = (info.downloaded - before) as f64 / seconds;
                            WEIGHT * measured + (1.0 - WEIGHT) * smooth
                        } else {
                            smooth
                        }
                    }
                    _ => 0.0,
                }
            } else {
                0.0
            };
            // Keep the older time stamp when the list came too soon to measure.
            let stamp = match self.last.get(&info.id) {
                Some(&(before, at, _))
                    if now.saturating_duration_since(at).as_secs_f64() < 0.2
                        && info.downloaded >= before =>
                {
                    (before, at)
                }
                _ => (info.downloaded, now),
            };
            next.insert(info.id, (stamp.0, stamp.1, speed));
            rows.push(row(info, speed.round() as u64));
        }
        self.last = next;
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn info(id: i64, status: &str, downloaded: u64) -> DownloadInfo {
        DownloadInfo {
            id,
            url: format!("https://x.test/{id}.bin"),
            filename: None,
            status: status.into(),
            downloaded,
            size: Some(10_000),
            error: None,
            path: None,
            page: None,
            expired: false,
        }
    }

    #[test]
    fn speed_is_measured_between_lists_and_gives_a_time_left() {
        let mut t = SpeedTracker::new();
        let start = Instant::now();
        let first = t.rows(vec![info(1, "downloading", 0)], start);
        assert_eq!((first[0].speed, first[0].eta), (0, None));
        let second = t.rows(
            vec![info(1, "downloading", 1000)],
            start + Duration::from_secs(1),
        );
        assert!(
            second[0].speed > 0 && second[0].speed <= 1000,
            "{}",
            second[0].speed
        );
        assert!(second[0].eta.is_some());
        assert_eq!(second[0].percent, Some(10.0));
    }

    #[test]
    fn only_running_downloads_have_a_speed() {
        let mut t = SpeedTracker::new();
        let start = Instant::now();
        t.rows(vec![info(1, "downloading", 0)], start);
        let rows = t.rows(
            vec![info(1, "paused", 5000)],
            start + Duration::from_secs(1),
        );
        assert_eq!((rows[0].speed, rows[0].eta), (0, None));
    }

    #[test]
    fn a_restart_from_zero_does_not_give_a_negative_or_huge_speed() {
        let mut t = SpeedTracker::new();
        let start = Instant::now();
        t.rows(vec![info(1, "downloading", 5000)], start);
        let rows = t.rows(
            vec![info(1, "downloading", 0)],
            start + Duration::from_secs(1),
        );
        assert_eq!(rows[0].speed, 0);
    }

    #[test]
    fn removed_downloads_are_forgotten() {
        let mut t = SpeedTracker::new();
        let start = Instant::now();
        t.rows(
            vec![info(1, "downloading", 0), info(2, "downloading", 0)],
            start,
        );
        t.rows(
            vec![info(2, "downloading", 100)],
            start + Duration::from_secs(1),
        );
        assert_eq!(t.last.len(), 1);
    }
}
