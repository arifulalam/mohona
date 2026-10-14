use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mohona_net::{HttpClient, Probe};
use tokio::fs::{self, File};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc::Sender;

use crate::filename::{sanitize_filename, unique_path};
use crate::Error;

const WRITE_BUFFER: usize = 256 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// What to download and where to put it.
#[derive(Clone, Debug)]
pub struct DownloadRequest {
    /// The address to download.
    pub url: String,
    /// The folder for the finished file. It is created if it does not exist.
    pub dest_dir: PathBuf,
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
    },
    /// Bytes received so far. May be skipped if the receiver is slow.
    Progress {
        /// Bytes written so far.
        downloaded: u64,
        /// Size in bytes, if known.
        total: Option<u64>,
    },
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
}

/// Download one URL as a single stream.
///
/// The data is written to `<name>.part` and renamed once it is complete and
/// the size matches, so a finished file is never half-written. If the
/// transfer fails, the `.part` file is removed (resuming arrives with the
/// progress ledger in a later step). An existing file is never overwritten:
/// the new one is numbered, like `name (1).ext`.
///
/// Progress is reported through `events` if given. Dropped progress events
/// are fine; `Started` and `Finished` are always delivered.
pub async fn download(
    client: &HttpClient,
    request: &DownloadRequest,
    events: Option<&Sender<Event>>,
) -> Result<Outcome, Error> {
    let probe = client.probe(&request.url).await?;
    let filename = sanitize_filename(probe.filename.as_deref().unwrap_or(""));

    fs::create_dir_all(&request.dest_dir).await?;
    let final_path = unique_path(&request.dest_dir, &filename);
    let part_path = part_path_for(&final_path);

    emit(
        events,
        Event::Started {
            url: probe.final_url.clone(),
            filename,
            total: probe.size,
            resumable: probe.accepts_ranges,
        },
    )
    .await;

    let bytes = match fetch_to_part(client, &probe, &part_path, events).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&part_path).await;
            return Err(error);
        }
    };

    if let Err(error) = fs::rename(&part_path, &final_path).await {
        let _ = fs::remove_file(&part_path).await;
        return Err(error.into());
    }

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
    })
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
    events: Option<&Sender<Event>>,
) -> Result<u64, Error> {
    let mut body = client.get(&probe.final_url).await?;
    let total = body.content_length().or(probe.size);

    let file = File::create(part_path).await?;
    let mut writer = BufWriter::with_capacity(WRITE_BUFFER, file);
    let mut downloaded = 0u64;
    let mut last_report = Instant::now();

    while let Some(chunk) = body.chunk().await? {
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

async fn emit(events: Option<&Sender<Event>>, event: Event) {
    if let Some(sender) = events {
        let _ = sender.send(event).await;
    }
}

fn report_progress(events: Option<&Sender<Event>>, downloaded: u64, total: Option<u64>) {
    if let Some(sender) = events {
        let _ = sender.try_send(Event::Progress { downloaded, total });
    }
}
