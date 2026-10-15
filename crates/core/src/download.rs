use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mohona_net::{HttpClient, Probe};
use tokio::fs::{self, File};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc::Sender;

use crate::cancel::CancelToken;
use crate::filename::{sanitize_filename, unique_path, unique_path_avoiding};
use crate::ledger::Ledger;
use crate::parallel::{fetch_segmented, Job, MIN_SEGMENT};
use crate::retry::{probe_with_retry, RetryPolicy};
use crate::Error;

const WRITE_BUFFER: usize = 256 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// How many connections a download uses unless told otherwise.
pub const DEFAULT_CONNECTIONS: usize = 8;
/// The most connections one download may use.
pub const MAX_CONNECTIONS: usize = 32;

/// What to download and where to put it.
#[derive(Clone, Debug)]
pub struct DownloadRequest {
    /// The address to download.
    pub url: String,
    /// The folder for the finished file. It is created if it does not exist.
    pub dest_dir: PathBuf,
    /// The most connections to use at once (1 to [`MAX_CONNECTIONS`]; values
    /// outside that are adjusted). Only used when the server supports byte
    /// ranges and the file is big enough to be worth splitting.
    pub connections: usize,
    /// How to deal with failing requests.
    pub retry: RetryPolicy,
    /// Call [`CancelToken::cancel`] on a clone of this to stop the download
    /// cleanly. What was downloaded is kept and can be resumed.
    pub cancel: CancelToken,
}

impl DownloadRequest {
    /// A request with the default number of connections.
    pub fn new(url: impl Into<String>, dest_dir: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            dest_dir: dest_dir.into(),
            connections: DEFAULT_CONNECTIONS,
            retry: RetryPolicy::default(),
            cancel: CancelToken::new(),
        }
    }
}

/// Something that happened during a download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The URL was probed and the transfer is about to begin.
    Started {
        /// The address after redirects.
        url: String,
        /// The safe file name the result will get (before any numbering).
        filename: String,
        /// Size in bytes, if known.
        total: Option<u64>,
        /// Whether the server supports resuming with byte ranges.
        resumable: bool,
        /// Bytes already on disk from an earlier, interrupted attempt that
        /// this download continues. Zero for a fresh download.
        resumed: u64,
        /// The most connections the download may use. It measures the
        /// server and often uses fewer.
        connections: usize,
    },
    /// Bytes received so far. May be skipped if the receiver is slow.
    Progress {
        /// Bytes written so far.
        downloaded: u64,
        /// Size in bytes, if known.
        total: Option<u64>,
    },
    /// The server pushed back (for example with 429 Too Many Requests), so
    /// the download now uses fewer connections.
    Limited {
        /// The most connections the download will use from now on.
        connections: usize,
    },
    /// A message about what the engine is deciding (for example why it
    /// uses fewer connections). Meant for a verbose mode.
    Note(String),
    /// The file is complete and in place.
    Finished {
        /// Where the file was saved.
        path: PathBuf,
        /// Size of the file in bytes.
        bytes: u64,
    },
}

/// The result of a finished download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Where the file was saved.
    pub path: PathBuf,
    /// Size of the file in bytes.
    pub bytes: u64,
    /// The address the file came from, after redirects.
    pub final_url: String,
    /// How many connections were opened for the file's data, including
    /// ones opened later to take over half of a slow segment.
    pub connections: usize,
}

/// Download one URL.
///
/// When the server supports byte ranges and the size is known, the file is
/// fetched with byte-range requests over as many connections as are worth it
/// (see `parallel` and `tuner`). Otherwise it is fetched as one stream.
///
/// The data is written to `<name>.part` and renamed once it is complete and
/// the size matches, so a finished file is never half-written. An existing
/// file is never overwritten: the new one is numbered, like `name (1).ext`.
///
/// **Resuming.** While a ranged download runs, its progress is saved to
/// `<name>.part.mohona` (see `ledger`). If the download fails or is cancelled
/// after some data was saved, both files are kept. Calling `download` again
/// with the same URL and folder finds them and continues, as long as the
/// server still reports the same size and `ETag`/`Last-Modified`; otherwise
/// the old data is discarded and the download starts over. Downloads without
/// range support cannot resume, and their partial file is removed on failure.
///
/// Progress is reported through `events` if given. Dropped progress events
/// are fine; `Started` and `Finished` are always delivered.
pub async fn download(
    client: &HttpClient,
    request: &DownloadRequest,
    events: Option<&Sender<Event>>,
) -> Result<Outcome, Error> {
    if request.cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let probe = probe_with_retry(client, &request.url, &request.retry).await?;
    let filename = sanitize_filename(probe.filename.as_deref().unwrap_or(""));
    fs::create_dir_all(&request.dest_dir).await?;

    let wanted = request.connections.clamp(1, MAX_CONNECTIONS);
    // Ranged downloads can be resumed, so they are used whenever they work,
    // even with one connection. Tiny files are not worth the bookkeeping.
    let ranged_total = probe
        .size
        .filter(|size| probe.accepts_ranges && *size >= MIN_SEGMENT);
    let url_key = request.url.trim().to_string();

    // An unfinished download of this URL? Use it if it is still valid.
    let mut earlier = None;
    if let Some((part, ledger)) = Ledger::find(&request.dest_dir, &url_key) {
        if ranged_total.is_some_and(|total| can_resume(&ledger, &probe, total, &part)) {
            earlier = Some((part, ledger));
        } else {
            // The file changed on the server (or cannot be resumed any more).
            let _ = fs::remove_file(Ledger::path_for(&part)).await;
            let _ = fs::remove_file(&part).await;
            emit(
                events,
                Event::Note(
                    "the earlier partial download no longer matches the server; starting over"
                        .into(),
                ),
            )
            .await;
        }
    }

    let (final_path, part_path, saved) = match &earlier {
        Some((part, ledger)) => (
            // Sanitized again: the ledger is a file anyone could have edited.
            request.dest_dir.join(sanitize_filename(&ledger.filename)),
            part.clone(),
            Some(ledger.segments.as_slice()),
        ),
        None => {
            let final_path = unique_path_avoiding(&request.dest_dir, &filename, &[".part"]);
            let part_path = part_path_for(&final_path);
            (final_path, part_path, None)
        }
    };
    let ledger_path = Ledger::path_for(&part_path);
    let resumed = earlier
        .as_ref()
        .map_or(0, |(_, ledger)| ledger.committed_bytes());

    let planned = ranged_total.map_or(1, |size| {
        usize::try_from(size / MIN_SEGMENT).map_or(wanted, |most| wanted.min(most))
    });
    emit(
        events,
        Event::Started {
            url: probe.final_url.clone(),
            filename: final_path.file_name().map_or_else(
                || filename.clone(),
                |name| name.to_string_lossy().into_owned(),
            ),
            total: probe.size,
            resumable: probe.accepts_ranges,
            resumed,
            connections: planned,
        },
    )
    .await;

    let fetched = match ranged_total {
        Some(total) => {
            let template = Ledger {
                version: 1,
                url: url_key,
                final_url: probe.final_url.clone(),
                filename: final_path.file_name().map_or_else(
                    || filename.clone(),
                    |name| name.to_string_lossy().into_owned(),
                ),
                total,
                etag: probe.etag.clone(),
                last_modified: probe.last_modified.clone(),
                segments: Vec::new(),
            };
            fetch_segmented(
                &Job {
                    client,
                    url: &probe.final_url,
                    etag: probe.etag.as_deref(),
                    total,
                    connections: wanted,
                    policy: &request.retry,
                    part_path: &part_path,
                    resume: saved,
                    ledger: &template,
                    ledger_path: &ledger_path,
                    cancel: &request.cancel,
                },
                events,
            )
            .await
            .map(|fetched| (fetched.bytes, fetched.connections))
            .map_err(|failed| (failed.error, failed.keep))
        }
        None => fetch_to_part(client, &probe, &part_path, &request.cancel, events)
            .await
            .map(|bytes| (bytes, 1))
            .map_err(|error| (error, false)),
    };
    let (bytes, connections) = match fetched {
        Ok(done) => done,
        Err((error, keep)) => {
            if !keep {
                let _ = fs::remove_file(&part_path).await;
                let _ = fs::remove_file(&ledger_path).await;
            }
            return Err(error);
        }
    };

    // A file with this name may have appeared while we were downloading.
    let final_path = if final_path.exists() {
        let name = final_path.file_name().map_or_else(
            || filename.clone(),
            |name| name.to_string_lossy().into_owned(),
        );
        unique_path(&request.dest_dir, &name)
    } else {
        final_path
    };
    if let Err(error) = fs::rename(&part_path, &final_path).await {
        let _ = fs::remove_file(&part_path).await;
        let _ = fs::remove_file(&ledger_path).await;
        return Err(error.into());
    }
    let _ = fs::remove_file(&ledger_path).await;

    emit(
        events,
        Event::Finished {
            path: final_path.clone(),
            bytes,
        },
    )
    .await;
    Ok(Outcome {
        path: final_path,
        bytes,
        final_url: probe.final_url,
        connections,
    })
}

/// What an interrupted download of `url` in `dest_dir` has saved, if anything:
/// `(bytes on disk, total size)`. Running [`download`] again continues from
/// there, if the server still has the same file.
pub fn partial_progress(dest_dir: &Path, url: &str) -> Option<(u64, u64)> {
    Ledger::find(dest_dir, url.trim()).map(|(_, ledger)| (ledger.committed_bytes(), ledger.total))
}

/// Whether an earlier attempt can be continued: the server still describes
/// the same file, and the partial file has the size the ledger expects.
fn can_resume(ledger: &Ledger, probe: &Probe, total: u64, part_path: &Path) -> bool {
    let same = |saved: &Option<String>, now: &Option<String>| match (saved, now) {
        (Some(saved), Some(now)) => saved == now,
        _ => true,
    };
    ledger.total == total
        && same(&ledger.etag, &probe.etag)
        && same(&ledger.last_modified, &probe.last_modified)
        && std::fs::metadata(part_path).is_ok_and(|meta| meta.len() == total)
}

fn part_path_for(final_path: &Path) -> PathBuf {
    let mut name = final_path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

async fn fetch_to_part(
    client: &HttpClient,
    probe: &Probe,
    part_path: &Path,
    cancel: &CancelToken,
    events: Option<&Sender<Event>>,
) -> Result<u64, Error> {
    let mut body = client.get(&probe.final_url).await?;
    let total = body.content_length().or(probe.size);

    let file = File::create(part_path).await?;
    let mut writer = BufWriter::with_capacity(WRITE_BUFFER, file);
    let mut downloaded = 0u64;
    let mut last_report = Instant::now();

    while let Some(chunk) = body.chunk().await? {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        writer.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if last_report.elapsed() >= PROGRESS_INTERVAL {
            last_report = Instant::now();
            report_progress(events, downloaded, total);
        }
    }
    writer.flush().await?;
    // Turn the file back into a plain one and drop it right here, so the
    // handle is closed before the rename. Windows refuses to rename open files.
    drop(writer.into_inner().into_std().await);

    if let Some(expected) = total {
        if expected != downloaded {
            return Err(Error::Incomplete {
                expected,
                received: downloaded,
            });
        }
    }
    report_progress(events, downloaded, total);
    Ok(downloaded)
}

pub(crate) async fn emit(events: Option<&Sender<Event>>, event: Event) {
    if let Some(sender) = events {
        let _ = sender.send(event).await;
    }
}

pub(crate) fn report_progress(events: Option<&Sender<Event>>, downloaded: u64, total: Option<u64>) {
    if let Some(sender) = events {
        let _ = sender.try_send(Event::Progress { downloaded, total });
    }
}
