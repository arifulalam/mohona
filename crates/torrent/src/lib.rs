//! BitTorrent downloads, for magnet links and `.torrent` addresses.
//!
//! The engine is [librqbit]. This layer gives it the shape the rest of Mohona
//! uses: start from an address, report progress, stop when asked, and say
//! where the finished files are. A finished torrent is not seeded: it is
//! removed from the session as soon as it is complete (the files stay).
//!
//! [librqbit]: https://crates.io/crates/librqbit

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions, TorrentStatsState,
};
use mohona_core::CancelToken;
use tokio::sync::OnceCell;

/// Whether an address is for this crate: a magnet link, or a web address of a `.torrent` file.
pub fn is_torrent_address(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("magnet:?") {
        return true;
    }
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && lower
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.ends_with(".torrent"))
}

/// What a running torrent reports, about twice a second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    /// The torrent's name, once it is known (a magnet link first has to find its description).
    pub name: Option<String>,
    /// Bytes on disk.
    pub downloaded: u64,
    /// Total size, once known.
    pub total: Option<u64>,
    /// Bytes per second from all peers.
    pub speed: u64,
    /// Peers connected now.
    pub peers: u32,
}

/// A finished torrent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub name: String,
    /// The file (one-file torrent) or folder (several files).
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Debug)]
pub enum TorrentError {
    /// Stopped by the person (the files stay).
    Cancelled,
    /// Failed, with a reason to show.
    Failed(String),
}

impl std::fmt::Display for TorrentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TorrentError::Cancelled => write!(f, "stopped"),
            TorrentError::Failed(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for TorrentError {}

fn failed(error: impl std::fmt::Display) -> TorrentError {
    TorrentError::Failed(format!("{error:#}"))
}

/// What to download.
#[derive(Clone)]
pub enum Source {
    /// A magnet link, or the web address of a `.torrent` file.
    Address(String),
    /// A `.torrent` file's contents.
    File(Vec<u8>),
}

/// One file of a torrent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Its number in the torrent (what `only` takes).
    pub index: u32,
    /// Its path inside the torrent, with `/` between folders.
    pub path: String,
    pub size: u64,
}

/// What a torrent holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub name: String,
    pub files: Vec<FileEntry>,
}

/// The torrent engine of one Mohona process. The session (its sockets, DHT
/// and peer table) is started when the first torrent is.
pub struct Torrents {
    state_dir: PathBuf,
    session: OnceCell<Arc<Session>>,
}

impl Torrents {
    /// `state_dir` is where the engine keeps its own small files (the DHT's address table).
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            session: OnceCell::new(),
        }
    }

    async fn session(&self) -> Result<&Arc<Session>, TorrentError> {
        self.session
            .get_or_try_init(|| async {
                let _ = std::fs::create_dir_all(&self.state_dir);
                // A computer without IPv6 cannot open the usual sockets: use IPv4 only then.
                let mut last = None;
                for ipv4_only in [false, true] {
                    let options = SessionOptions {
                        listen: Some(librqbit::ListenerOptions {
                            ipv4_only,
                            ..librqbit::ListenerOptions::default()
                        }),
                        ipv4_only,
                        ..SessionOptions::default()
                    };
                    match Session::new_with_opts(self.state_dir.clone(), options).await {
                        Ok(session) => return Ok(session),
                        Err(error) => last = Some(error),
                    }
                }
                Err(failed(last.expect("two tries were made")))
            })
            .await
    }

    /// What the torrent holds, without downloading anything. A magnet link first has to find its
    /// description from peers, which can take a while: `cancel` ends the wait.
    pub async fn list(
        &self,
        source: Source,
        peers: Vec<SocketAddr>,
        cancel: &CancelToken,
    ) -> Result<Listing, TorrentError> {
        let session = self.session().await?;
        let add = match source {
            Source::Address(url) => AddTorrent::from_url(url),
            Source::File(bytes) => AddTorrent::from_bytes(bytes),
        };
        let options = AddTorrentOptions {
            list_only: true,
            initial_peers: (!peers.is_empty()).then_some(peers),
            ..AddTorrentOptions::default()
        };
        let added = tokio::select! {
            () = cancel.cancelled() => return Err(TorrentError::Cancelled),
            added = session.add_torrent(add, Some(options)) => added.map_err(failed)?,
        };
        let AddTorrentResponse::ListOnly(listed) = added else {
            return Err(TorrentError::Failed("the torrent was not listed".into()));
        };
        let info = &listed.info;
        let files: Vec<FileEntry> = info
            .iter_file_details()
            .enumerate()
            .map(|(i, f)| FileEntry {
                index: u32::try_from(i).unwrap_or(u32::MAX),
                path: f.filename.to_string(),
                size: f.len,
            })
            .collect();
        let name = info
            .name()
            .map(|n| n.into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "torrent".into());
        Ok(Listing { name, files })
    }

    /// Download into `dest` until done, cancelled or failed. `peers` are tried
    /// first (used by tests and for magnet links that name peers).
    pub async fn run(
        &self,
        source: Source,
        dest: &Path,
        peers: Vec<SocketAddr>,
        only: Option<Vec<u32>>,
        cancel: &CancelToken,
        mut report: impl FnMut(Update) + Send,
    ) -> Result<Done, TorrentError> {
        let session = self.session().await?;
        // The engine puts the files of a torrent straight into the folder it is given, so a torrent
        // of several files gets a folder of its own (its name) instead of spilling into `dest`.
        let listing = self.list(source.clone(), peers.clone(), cancel).await?;
        let root = if listing.files.len() > 1 {
            dest.join(folder_name(&listing.name))
        } else {
            dest.to_path_buf()
        };
        let many = listing.files.len() > 1;
        let dest = root.as_path();
        std::fs::create_dir_all(dest).map_err(failed)?;
        let add = match source {
            Source::Address(url) => AddTorrent::from_url(url),
            Source::File(bytes) => AddTorrent::from_bytes(bytes),
        };
        let options = AddTorrentOptions {
            // Continue over files that are already there (a resumed download).
            overwrite: true,
            output_folder: Some(dest.display().to_string()),
            initial_peers: (!peers.is_empty()).then_some(peers),
            // Only the chosen files (all of them when none were chosen).
            only_files: only.map(|o| o.into_iter().map(|i| i as usize).collect()),
            ..AddTorrentOptions::default()
        };
        let added = tokio::select! {
            () = cancel.cancelled() => return Err(TorrentError::Cancelled),
            added = session.add_torrent(add, Some(options)) => added.map_err(failed)?,
        };
        let (id, handle) = match added {
            AddTorrentResponse::Added(id, handle)
            | AddTorrentResponse::AlreadyManaged(id, handle) => (id, handle),
            AddTorrentResponse::ListOnly(_) => {
                return Err(TorrentError::Failed("the torrent was only listed".into()))
            }
        };
        // Leave the session; the files stay where they are.
        let forget = || async move {
            let _ = session
                .delete(librqbit::api::TorrentIdOrHash::Id(id), false)
                .await;
        };
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                () = cancel.cancelled() => {
                    forget().await;
                    return Err(TorrentError::Cancelled);
                }
                _ = tick.tick() => {}
            }
            let stats = handle.stats();
            let name = handle.name();
            let total = (stats.total_bytes > 0).then_some(stats.total_bytes);
            let (speed, peers) = stats.live.as_ref().map_or((0, 0), |live| {
                let p = &live.snapshot.peer_stats;
                (live.download_speed.as_bytes(), p.live)
            });
            report(Update {
                name: name.clone(),
                downloaded: stats.progress_bytes,
                total,
                speed,
                peers,
            });
            if let (TorrentStatsState::Error, message) = (&stats.state, &stats.error) {
                forget().await;
                return Err(TorrentError::Failed(
                    message
                        .clone()
                        .unwrap_or_else(|| "the torrent failed".into()),
                ));
            }
            if stats.finished {
                forget().await;
                let name = name.unwrap_or_else(|| "torrent".into());
                let inside = dest.join(&name);
                let path = if many || !inside.exists() {
                    dest.to_path_buf()
                } else {
                    inside
                };
                return Ok(Done {
                    name,
                    path,
                    bytes: stats.total_bytes,
                });
            }
        }
    }
}

/// A torrent's name as one safe folder name.
fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim().to_string();
    if cleaned.is_empty() {
        "torrent".into()
    } else {
        cleaned.chars().take(120).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_torrents_name_becomes_one_safe_folder_name() {
        assert_eq!(folder_name("Album"), "Album");
        assert_eq!(folder_name("../../etc"), "etc");
        assert_eq!(folder_name("a\\b\\c"), "c");
        assert_eq!(folder_name(".."), "torrent");
        assert_eq!(folder_name("what?*:name."), "what___name");
        assert_eq!(folder_name(""), "torrent");
    }

    #[test]
    fn magnet_links_and_torrent_files_are_recognised() {
        assert!(is_torrent_address("magnet:?xt=urn:btih:abc"));
        assert!(is_torrent_address("  MAGNET:?xt=urn:btih:abc"));
        assert!(is_torrent_address("https://x.test/a/b.torrent"));
        assert!(is_torrent_address("http://x.test/a.TORRENT?key=1#top"));
        assert!(!is_torrent_address("https://x.test/a.zip"));
        assert!(!is_torrent_address("https://x.test/a.zip?name=b.torrent"));
        assert!(!is_torrent_address("ftp://x.test/a.torrent"));
        assert!(!is_torrent_address("magnet:xt=urn"));
    }
}
