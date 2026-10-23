//! The progress ledger: a small file next to the `.part` file that records
//! which parts of the download are safely on disk, so an interrupted download
//! can continue instead of starting over.
//!
//! For every segment the ledger stores `(start, committed, end)`: the segment
//! covers bytes `start..end`, and `start..committed` of it are written. The
//! segments always cover the whole file without gaps or overlaps.
//!
//! A ledger is only written after the `.part` file has been flushed to disk,
//! and it is replaced atomically (write a temporary file, then rename), so a
//! crash or power cut leaves either the old ledger or the new one, never a
//! half-written one, and never one that claims bytes that are not on disk.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
/// Added to the `.part` path to get the ledger path.
const SUFFIX: &str = ".mohona";

/// What is known about a download in progress.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u32,
    /// The address as the user gave it. Used to find this ledger again.
    pub url: String,
    /// The address after redirects, which the data is fetched from.
    pub final_url: String,
    /// The file name the result gets (before any numbering at the end).
    pub filename: String,
    /// Size of the whole file.
    pub total: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// `(start, committed, end)` of each segment.
    pub segments: Vec<(u64, u64, u64)>,
}

/// Where a download's progress is kept. By default it is a sidecar file next
/// to the `.part` file; an application can provide its own (the manager keeps
/// it in its database) through `DownloadRequest::progress`.
///
/// Implementations must make `save` atomic: after a crash, `load` returns
/// either the previous snapshot or the new one, never a mix. `save` is
/// called from a blocking thread and may take its time.
pub trait ProgressStore: Send + Sync + std::fmt::Debug {
    /// The saved progress of this download, if any.
    fn load(&self) -> Option<Ledger>;
    /// Replace the saved progress.
    fn save(&self, progress: &Ledger) -> io::Result<()>;
    /// Forget the saved progress (the download finished or was discarded).
    fn clear(&self);
}

/// Where checkpoints go during one download.
#[derive(Clone, Debug)]
pub(crate) enum Sink {
    /// The sidecar file at this path.
    File(PathBuf),
    /// An application-provided store. The path is the sidecar file of the
    /// same download, removed when finished (it may exist from an older run).
    Store(Arc<dyn ProgressStore>, PathBuf),
}

impl Sink {
    pub fn save(&self, ledger: &Ledger) -> io::Result<()> {
        match self {
            Sink::File(path) => ledger.save(path),
            Sink::Store(store, _) => store.save(ledger),
        }
    }

    pub fn clear(&self) {
        match self {
            Sink::File(path) => {
                let _ = fs::remove_file(path);
            }
            Sink::Store(store, sidecar) => {
                store.clear();
                let _ = fs::remove_file(sidecar);
            }
        }
    }
}

impl Ledger {
    /// The ledger belonging to a `.part` file.
    pub fn path_for(part_path: &Path) -> PathBuf {
        let mut name = part_path.as_os_str().to_owned();
        name.push(SUFFIX);
        PathBuf::from(name)
    }

    /// Bytes that are safely on disk.
    pub fn committed_bytes(&self) -> u64 {
        self.segments
            .iter()
            .map(|(start, committed, _)| committed - start)
            .sum()
    }

    /// How many bytes from the start of the file are complete, with nothing
    /// missing before them: what a player can use of a partial file.
    pub fn contiguous_bytes(&self) -> u64 {
        let mut sorted = self.segments.clone();
        sorted.sort_unstable();
        let mut prefix = 0;
        for (start, committed, end) in sorted {
            if start != prefix {
                break;
            }
            prefix = committed;
            if committed < end {
                break;
            }
        }
        prefix
    }

    /// Whether the segments make sense: they tile `0..total` exactly and
    /// no segment has progress outside itself.
    pub fn is_consistent(&self) -> bool {
        if self.version != VERSION || self.segments.is_empty() {
            return false;
        }
        let mut sorted = self.segments.clone();
        sorted.sort_unstable();
        let mut expected_start = 0;
        for (start, committed, end) in sorted {
            if start != expected_start || !(start <= committed && committed <= end) {
                return false;
            }
            expected_start = end;
        }
        expected_start == self.total
    }

    /// Read a ledger. Anything missing, unreadable or inconsistent gives
    /// `None`: the caller then simply starts the download from the beginning.
    pub fn load(path: &Path) -> Option<Self> {
        let text = fs::read(path).ok()?;
        let ledger: Ledger = serde_json::from_slice(&text).ok()?;
        ledger.is_consistent().then_some(ledger)
    }

    /// Write the ledger atomically.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // Replaces an existing ledger in one step (also on Windows).
        fs::rename(&temporary, path)
    }

    /// Find an unfinished download of `url` in `dir`: its `.part` file and
    /// ledger. Both must exist.
    pub fn find(dir: &Path, url: &str) -> Option<(PathBuf, Ledger)> {
        let mut candidates: Vec<PathBuf> = fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".part.mohona"))
            })
            .collect();
        candidates.sort();
        candidates.into_iter().find_map(|ledger_path| {
            let ledger = Ledger::load(&ledger_path)?;
            if ledger.url != url {
                return None;
            }
            let part = ledger_path.to_str()?.strip_suffix(SUFFIX)?;
            let part_path = PathBuf::from(part);
            part_path.is_file().then_some((part_path, ledger))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Ledger;

    fn sample(segments: Vec<(u64, u64, u64)>) -> Ledger {
        Ledger {
            version: 1,
            url: "http://example.com/a.bin".into(),
            final_url: "http://example.com/a.bin".into(),
            filename: "a.bin".into(),
            total: 100,
            etag: Some("\"x\"".into()),
            last_modified: None,
            segments,
        }
    }

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin.part.mohona");
        let ledger = sample(vec![(0, 30, 50), (50, 50, 100)]);
        ledger.save(&path).unwrap();
        let loaded = Ledger::load(&path).unwrap();
        assert_eq!(loaded.segments, ledger.segments);
        assert_eq!(loaded.committed_bytes(), 30);
        assert!(!dir.path().join("a.bin.part.mohona.tmp").exists());
    }

    #[test]
    fn saving_again_replaces_the_old_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.part.mohona");
        sample(vec![(0, 10, 100)]).save(&path).unwrap();
        sample(vec![(0, 90, 100)]).save(&path).unwrap();
        assert_eq!(Ledger::load(&path).unwrap().committed_bytes(), 90);
    }

    #[test]
    fn damaged_or_inconsistent_ledgers_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.part.mohona");
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(Ledger::load(&path).is_none());
        assert!(Ledger::load(&dir.path().join("missing")).is_none());

        // A gap, an overlap, progress beyond the segment, a short total.
        for bad in [
            vec![(0, 0, 40), (50, 50, 100)],
            vec![(0, 0, 60), (50, 50, 100)],
            vec![(0, 70, 50), (50, 50, 100)],
            vec![(0, 0, 50)],
            vec![],
        ] {
            sample(bad).save(&path).unwrap();
            assert!(Ledger::load(&path).is_none());
        }
        // Order does not matter.
        sample(vec![(50, 60, 100), (0, 0, 50)]).save(&path).unwrap();
        assert!(Ledger::load(&path).is_some());
    }

    #[test]
    fn contiguous_bytes_stop_at_the_first_gap() {
        assert_eq!(sample(vec![(0, 100, 100)]).contiguous_bytes(), 100);
        assert_eq!(
            sample(vec![(0, 40, 50), (50, 100, 100)]).contiguous_bytes(),
            40
        );
        assert_eq!(
            sample(vec![(50, 100, 100), (0, 50, 50)]).contiguous_bytes(),
            100
        );
        assert_eq!(
            sample(vec![(0, 0, 50), (50, 100, 100)]).contiguous_bytes(),
            0
        );
    }

    #[test]
    fn finds_the_ledger_for_a_url_only_with_its_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let ledger_path = dir.path().join("a.bin.part.mohona");
        sample(vec![(0, 10, 100)]).save(&ledger_path).unwrap();
        assert!(Ledger::find(dir.path(), "http://example.com/a.bin").is_none());

        std::fs::write(dir.path().join("a.bin.part"), b"x").unwrap();
        let (part, ledger) = Ledger::find(dir.path(), "http://example.com/a.bin").unwrap();
        assert_eq!(part, dir.path().join("a.bin.part"));
        assert_eq!(ledger.filename, "a.bin");
        assert!(Ledger::find(dir.path(), "http://example.com/other").is_none());
    }
}
