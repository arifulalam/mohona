//! Downloading one file over several connections at once.
//!
//! The file is created at its full size first. Each worker asks the server
//! for one byte range and writes it at the matching place in the file, through
//! its own file handle. When a worker finishes early, the coordinator cuts the
//! biggest unfinished segment in half and gives the second half to a new
//! worker, so a slow connection never leaves the others idle at the end.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mohona_net::HttpClient;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;
use tokio::time::{interval, MissedTickBehavior};

use crate::download::{report_progress, Event};
use crate::segments::{Claim, Scheduler};
use crate::Error;

/// Smallest piece worth a connection of its own.
pub(crate) const MIN_SEGMENT: u64 = 512 * 1024;
const WRITE_BUFFER: usize = 256 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// What a worker needs to know, shared by all of them.
struct Shared {
    client: HttpClient,
    url: String,
    etag: Option<String>,
    part_path: std::path::PathBuf,
    scheduler: Scheduler,
    downloaded: AtomicU64,
}

/// What a finished parallel download did.
pub(crate) struct Fetched {
    /// Bytes in the file.
    pub bytes: u64,
    /// How many connections were opened for the body, counting the ones
    /// started after a segment was split.
    pub connections: usize,
}

/// Fill `part_path` with `total` bytes from `url` using up to `connections`
/// connections. The caller removes the file if this fails.
pub(crate) async fn fetch_segmented(
    client: &HttpClient,
    url: &str,
    etag: Option<&str>,
    total: u64,
    connections: usize,
    part_path: &Path,
    events: Option<&Sender<Event>>,
) -> Result<Fetched, Error> {
    // Create the file at full size now: a full disk fails here, before any
    // download time is spent, and workers can write anywhere in it.
    let file = tokio::fs::File::create(part_path).await?;
    file.set_len(total).await?;
    drop(file.into_std().await);

    let shared = Arc::new(Shared {
        client: client.clone(),
        url: url.to_string(),
        etag: etag.map(str::to_string),
        part_path: part_path.to_path_buf(),
        scheduler: Scheduler::new(total, connections, MIN_SEGMENT),
        downloaded: AtomicU64::new(0),
    });

    let mut workers: JoinSet<Result<(), Error>> = JoinSet::new();
    let mut opened = 0usize;
    for claim in shared.scheduler.initial_claims() {
        spawn_worker(&mut workers, &shared, claim);
        opened += 1;
    }

    let mut ticker = interval(PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let result = loop {
        tokio::select! {
            finished = workers.join_next() => {
                let Some(finished) = finished else { break Ok(()) };
                match finished {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => break Err(error),
                    Err(join_error) => break Err(Error::Io(std::io::Error::other(join_error))),
                }
                // A connection is free: give it half of the slowest segment.
                while workers.len() < connections {
                    let Some(claim) = shared.scheduler.steal() else { break };
                    spawn_worker(&mut workers, &shared, claim);
                    opened += 1;
                }
            }
            _ = ticker.tick() => {
                report_progress(events, shared.downloaded.load(Ordering::Relaxed), Some(total));
            }
        }
    };

    if let Err(error) = result {
        // Wait for the workers to really stop, so their file handles are
        // closed before the caller deletes the file (Windows needs that).
        workers.abort_all();
        while workers.join_next().await.is_some() {}
        return Err(error);
    }

    let bytes = shared.downloaded.load(Ordering::Relaxed);
    if bytes != total || shared.scheduler.remaining() != 0 {
        return Err(Error::Incomplete {
            expected: total,
            received: bytes,
        });
    }
    report_progress(events, bytes, Some(total));
    Ok(Fetched {
        bytes,
        connections: opened,
    })
}

fn spawn_worker(workers: &mut JoinSet<Result<(), Error>>, shared: &Arc<Shared>, claim: Claim) {
    let shared = Arc::clone(shared);
    workers.spawn(async move { run_worker(&shared, claim).await });
}

/// Download one segment. It stops when the segment is finished, or when the
/// scheduler has cut it short and the worker has reached its new end.
async fn run_worker(shared: &Shared, claim: Claim) -> Result<(), Error> {
    let mut body = shared
        .client
        .get_range(
            &shared.url,
            claim.start,
            claim.end - 1,
            shared.etag.as_deref(),
        )
        .await?;

    let mut file = OpenOptions::new()
        .write(true)
        .open(&shared.part_path)
        .await?;
    file.seek(std::io::SeekFrom::Start(claim.start)).await?;
    let mut writer = BufWriter::with_capacity(WRITE_BUFFER, file);

    'receive: while let Some(chunk) = body.chunk().await? {
        let mut data: &[u8] = &chunk;
        while !data.is_empty() {
            let allowed = shared.scheduler.reserve(claim.id, data.len() as u64) as usize;
            if allowed == 0 {
                break 'receive;
            }
            writer.write_all(&data[..allowed]).await?;
            shared
                .downloaded
                .fetch_add(allowed as u64, Ordering::Relaxed);
            data = &data[allowed..];
            if !data.is_empty() {
                // The segment ends here: the rest belongs to another worker.
                break 'receive;
            }
        }
    }
    writer.flush().await?;
    // Close the handle for real before returning (see `fetch_to_part`).
    drop(writer.into_inner().into_std().await);

    let (start, position, end) = shared.scheduler.state(claim.id);
    if position != end {
        return Err(Error::Incomplete {
            expected: end - start,
            received: position - start,
        });
    }
    Ok(())
}
