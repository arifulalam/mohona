//! Downloading one file over several connections at once.
//!
//! The file is created at its full size first. Each worker asks the server
//! for one byte range and writes it at the matching place in the file, through
//! its own file handle. When a worker finishes early, the coordinator cuts the
//! biggest unfinished segment in half and gives the second half to a new
//! worker, so a slow connection never leaves the others idle at the end.
//!
//! Failures are handled here too. A hiccup (a dropped connection, a 500) makes
//! the worker wait a little and continue from the byte where it stopped. When
//! the server pushes back (429, 503, refused connections), the worker gives its
//! segment back, the download lowers its number of connections, and the
//! segment is picked up again after a pause.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_net::{Failure, HttpClient, NetError};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc::Sender;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::{interval, sleep, MissedTickBehavior};

use crate::cancel::CancelToken;
use crate::download::{emit, report_progress, Event};
use crate::ledger::{Ledger, Sink};
use crate::limiter::Limits;
use crate::retry::RetryPolicy;
use crate::segments::{Claim, Scheduler};
use crate::tuner::{Action, Tuner};
use crate::Error;

/// Smallest piece worth a connection of its own.
pub(crate) const MIN_SEGMENT: u64 = 512 * 1024;
const WRITE_BUFFER: usize = 256 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// How often progress is saved to the ledger.
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(1);

/// What a worker needs to know, shared by all of them.
struct Shared {
    client: HttpClient,
    url: String,
    etag: Option<String>,
    part_path: PathBuf,
    policy: RetryPolicy,
    scheduler: Scheduler,
    limits: Limits,
    downloaded: AtomicU64,
    /// How many workers have been asked to stop because the tuner found
    /// their connection useless. Each worker that obeys takes one off.
    retire: AtomicUsize,
    /// Wakes workers that are waiting for a connection, so they can stop.
    wake: Notify,
}

impl Shared {
    /// Whether this worker should stop now (and, if so, uses up one request).
    fn should_retire(&self) -> bool {
        let mut asked = self.retire.load(Ordering::Relaxed);
        while asked > 0 {
            match self.retire.compare_exchange(
                asked,
                asked - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(now) => asked = now,
            }
        }
        false
    }
}

/// What a finished parallel download did.
pub(crate) struct Fetched {
    /// Bytes in the file.
    pub bytes: u64,
    /// How many connections were opened for the body, counting the ones
    /// started after a segment was split or taken over again.
    pub connections: usize,
}

/// How a worker ended without an error.
enum Ended {
    /// Its segment is complete (or was cut short and reached its new end).
    Done,
    /// It stopped on request. The segment is not finished.
    Retired { id: usize },
    /// The server pushed back. The segment is not finished.
    Throttled {
        id: usize,
        retry_after: Option<Duration>,
        error: Error,
    },
}

/// Why a download stopped, and whether its partial file is worth keeping.
pub(crate) struct FetchFailed {
    pub error: Error,
    /// Some data is safely on disk and recorded in the ledger, so the
    /// download can be resumed. When `false`, the caller deletes the files.
    pub keep: bool,
}

impl FetchFailed {
    fn discard(error: impl Into<Error>) -> Self {
        Self {
            error: error.into(),
            keep: false,
        }
    }
}

/// What to fetch and where to put it.
pub(crate) struct Job<'a> {
    pub client: &'a HttpClient,
    /// The address to download from (after redirects).
    pub url: &'a str,
    /// Sent as `If-Range`, so a file that changed is noticed.
    pub etag: Option<&'a str>,
    /// Size of the file in bytes.
    pub total: u64,
    /// The most connections to use at once.
    pub connections: usize,
    pub policy: &'a RetryPolicy,
    pub part_path: &'a Path,
    /// The segments of an earlier, interrupted attempt, as `(start,
    /// committed, end)`. The `.part` file must already have the right size.
    pub resume: Option<&'a [(u64, u64, u64)]>,
    /// What is written to the ledger (its `segments` are filled in at each
    /// checkpoint) and where.
    pub ledger: &'a Ledger,
    pub sink: &'a Sink,
    pub limits: &'a Limits,
    pub cancel: &'a CancelToken,
}

/// Fill the job's `.part` file with its bytes using up to `job.connections`
/// connections. The caller removes the file if this fails.
pub(crate) async fn fetch_segmented(
    job: &Job<'_>,
    events: Option<&Sender<Event>>,
) -> Result<Fetched, FetchFailed> {
    let Job {
        client,
        url,
        etag,
        total,
        connections,
        policy,
        part_path,
        resume,
        ledger,
        sink,
        limits,
        cancel,
    } = *job;
    if resume.is_none() {
        // Create the file at full size now: a full disk fails here, before
        // any download time is spent, and workers can write anywhere in it.
        let create = async {
            let file = tokio::fs::File::create(part_path).await?;
            file.set_len(total).await?;
            drop(file.into_std().await);
            std::io::Result::Ok(())
        };
        create.await.map_err(FetchFailed::discard)?;
    }

    let shared = Arc::new(Shared {
        client: client.clone(),
        url: url.to_string(),
        etag: etag.map(str::to_string),
        part_path: part_path.to_path_buf(),
        policy: policy.clone(),
        limits: limits.clone(),
        // An adaptive download starts as one long request, like a browser's;
        // the tuner cuts it up when more connections prove worth it.
        scheduler: match resume {
            Some(saved) => Scheduler::restore(saved, MIN_SEGMENT),
            None => Scheduler::new(
                total,
                if policy.adaptive { 1 } else { connections },
                MIN_SEGMENT,
            ),
        },
        downloaded: AtomicU64::new(0),
        retire: AtomicUsize::new(0),
        wake: Notify::new(),
    });

    shared
        .downloaded
        .store(shared.scheduler.committed_bytes(), Ordering::Relaxed);
    // Record the download before any data arrives. If the program is killed
    // at any moment from here on, the next run finds the `.part` file and
    // continues with it instead of leaving it behind as an orphan.
    if let Err(error) = checkpoint(&shared, ledger, sink).await {
        emit(
            events,
            Event::Note(format!("could not save progress: {error}")),
        )
        .await;
    }

    let mut workers: JoinSet<Result<Ended, Error>> = JoinSet::new();
    let mut opened = 0usize;
    // Adaptive downloads start with one connection and let the tuner add more;
    // the other segments wait for it (or for a finished worker), see
    // `Scheduler::adopt`. Otherwise every connection starts at once.
    let start = if policy.adaptive { 1 } else { connections };
    for claim in shared.scheduler.initial_claims() {
        if workers.len() < start {
            spawn_worker(&mut workers, &shared, claim, Duration::ZERO);
            opened += 1;
        } else {
            shared.scheduler.release(claim.id);
        }
    }

    // How many connections may run at once. It starts at the request and only
    // ever goes down: when the server pushes back, or the tuner finds that
    // more connections do not help.
    let mut allowed = usize::try_from(total / MIN_SEGMENT)
        .unwrap_or(usize::MAX)
        .clamp(1, connections);
    let mut throttles_without_progress = 0u32;
    let mut progress_at_last_throttle = 0u64;

    let mut ticker = interval(PROGRESS_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut tuner = Tuner::new();
    if !policy.adaptive {
        tuner.stop();
    }
    let mut sampler = interval(policy.sample_interval.max(Duration::from_millis(1)));
    sampler.set_missed_tick_behavior(MissedTickBehavior::Delay);
    sampler.tick().await; // the first tick is immediate
    let mut last_sample = (Instant::now(), shared.downloaded.load(Ordering::Relaxed));
    let mut last_checkpoint = Instant::now();

    let result = loop {
        tokio::select! {
            finished = workers.join_next() => {
                let Some(finished) = finished else { break Ok(()) };
                let mut pause = Duration::ZERO;
                match finished {
                    Ok(Ok(Ended::Done)) => {}
                    Ok(Ok(Ended::Retired { id })) => shared.scheduler.release(id),
                    Ok(Ok(Ended::Throttled { id, retry_after, error })) => {
                        shared.scheduler.release(id);
                        tuner.stop();
                        emit(events, Event::Note(format!(
                            "the server pushed back ({error}); {} connection(s) still running",
                            workers.len()
                        ))).await;
                        let downloaded = shared.downloaded.load(Ordering::Relaxed);
                        if downloaded > progress_at_last_throttle {
                            throttles_without_progress = 0;
                            progress_at_last_throttle = downloaded;
                        }
                        // Keep only the connections that still work. A refusal
                        // that lowers the number is news, not a failure; only
                        // refusals that change nothing count toward giving up.
                        let lowered = allowed.min(workers.len().max(1));
                        if lowered < allowed {
                            allowed = lowered;
                            emit(events, Event::Limited { connections: allowed }).await;
                        } else {
                            throttles_without_progress += 1;
                            if throttles_without_progress > shared.policy.max_throttles {
                                break Err(error);
                            }
                        }
                        // Wait longer than for an ordinary hiccup: a server that
                        // pushes back should be given time, not poked again.
                        pause = shared
                            .policy
                            .backoff(throttles_without_progress.max(1) + 2, retry_after);
                    }
                    Ok(Err(error)) => break Err(error),
                    Err(join_error) => break Err(Error::Io(std::io::Error::other(join_error))),
                }
                // The freed connection takes the next waiting segment, or
                // half of the slowest running one.
                if workers.len() < allowed {
                    opened += usize::from(start_next(&mut workers, &shared, pause));
                }
            }
            _ = sampler.tick(), if !tuner.is_done() => {
                let now = Instant::now();
                let bytes = shared.downloaded.load(Ordering::Relaxed);
                let seconds = now.duration_since(last_sample.0).as_secs_f64().max(0.001);
                let speed = (bytes - last_sample.1) as f64 / seconds;
                last_sample = (now, bytes);
                match tuner.sample(speed, workers.len()) {
                    Action::None => {}
                    Action::Grow(wanted) => {
                        let room = allowed.saturating_sub(workers.len());
                        let mut started = 0;
                        while started < wanted.min(room)
                            && start_next(&mut workers, &shared, Duration::ZERO)
                        {
                            started += 1;
                        }
                        opened += started;
                        if started > 0 {
                            emit(events, Event::Note(format!(
                                "speed is {}/s with {} connection(s): trying {} more",
                                human_speed(tuner.last_mean()), workers.len() - started, started
                            ))).await;
                        }
                        tuner.grew(started);
                    }
                    Action::Retire(count) => {
                        let count = count.min(workers.len().saturating_sub(1));
                        shared.retire.fetch_add(count, Ordering::Relaxed);
                        shared.wake.notify_waiters();
                        emit(events, Event::Note(format!(
                            "more connections did not make it faster: going back to {}",
                            workers.len().max(1) - count
                        ))).await;
                        allowed = workers.len().max(1) - count;
                    }
                }
            }
            _ = ticker.tick() => {
                report_progress(events, shared.downloaded.load(Ordering::Relaxed), Some(total));
                if last_checkpoint.elapsed() >= CHECKPOINT_INTERVAL {
                    if let Err(error) = checkpoint(&shared, ledger, sink).await {
                        emit(events, Event::Note(format!("could not save progress: {error}"))).await;
                    }
                    last_checkpoint = Instant::now();
                }
            }
            () = cancel.cancelled() => break Err(Error::Cancelled),
        }
        // Requests to stop that nobody is left to obey would hit the wrong
        // worker later, so drop them once the count is where it should be.
        if workers.len() <= allowed {
            shared.retire.store(0, Ordering::Relaxed);
        }
    };

    if let Err(error) = result {
        // Let the workers write what they hold and stop, so their file
        // handles are closed before anything is renamed or deleted (Windows
        // needs that), and so the ledger can record their last bytes.
        workers.abort_all();
        while workers.join_next().await.is_some() {}
        return Err(finish_failed(&shared, ledger, sink, error).await);
    }

    let bytes = shared.downloaded.load(Ordering::Relaxed);
    if bytes != total || shared.scheduler.remaining() != 0 {
        let error = Error::Incomplete {
            expected: total,
            received: bytes,
        };
        return Err(finish_failed(&shared, ledger, sink, error).await);
    }
    // Make sure the bytes are on disk before the caller renames the file.
    sync_part_file(shared.part_path.clone())
        .await
        .map_err(FetchFailed::discard)?;
    report_progress(events, bytes, Some(total));
    Ok(Fetched {
        bytes,
        connections: opened,
    })
}

/// Start one more worker, on a waiting segment if there is one, otherwise on
/// half of the biggest running one. Returns whether a worker was started.
fn start_next(
    workers: &mut JoinSet<Result<Ended, Error>>,
    shared: &Arc<Shared>,
    delay: Duration,
) -> bool {
    if let Some(claim) = shared.scheduler.adopt() {
        spawn_worker(workers, shared, claim, delay);
    } else if let Some(claim) = shared.scheduler.steal() {
        spawn_worker(workers, shared, claim, Duration::ZERO);
    } else {
        return false;
    }
    true
}

/// Flush the `.part` file to disk.
async fn sync_part_file(path: PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || {
        // Write access is needed to flush on Windows.
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .sync_data()
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Save progress to the ledger.
///
/// The order matters: take the list of committed bytes first, then flush the
/// file, then write the ledger. Everything the ledger claims was written
/// before the snapshot, so the flush that follows makes it durable.
async fn checkpoint(shared: &Shared, template: &Ledger, sink: &Sink) -> std::io::Result<()> {
    let mut ledger = template.clone();
    ledger.segments = shared.scheduler.snapshot();
    sync_part_file(shared.part_path.clone()).await?;
    let sink = sink.clone();
    tokio::task::spawn_blocking(move || sink.save(&ledger))
        .await
        .map_err(std::io::Error::other)?
}

/// A download stopped early: record what is on disk, and say whether it is
/// worth keeping.
async fn finish_failed(
    shared: &Shared,
    template: &Ledger,
    sink: &Sink,
    error: Error,
) -> FetchFailed {
    let progress = shared.scheduler.committed_bytes() > 0;
    let keep = progress
        && !error.makes_partial_file_useless()
        && checkpoint(shared, template, sink).await.is_ok();
    FetchFailed { error, keep }
}

fn spawn_worker(
    workers: &mut JoinSet<Result<Ended, Error>>,
    shared: &Arc<Shared>,
    claim: Claim,
    delay: Duration,
) {
    let shared = Arc::clone(shared);
    workers.spawn(async move {
        if !delay.is_zero() {
            sleep(delay).await;
        }
        run_worker(&shared, claim.id).await
    });
}

/// Why one attempt at a segment stopped before the end.
enum Attempt {
    /// Nothing can fix this: give up on the whole download.
    Fatal(Error),
    /// Trying again may work.
    Retry {
        failure: Failure,
        retry_after: Option<Duration>,
        error: Error,
    },
}

impl From<NetError> for Attempt {
    fn from(error: NetError) -> Self {
        Attempt::Retry {
            failure: error.failure(),
            retry_after: error.retry_after(),
            error: error.into(),
        }
        .or_fatal()
    }
}

impl From<std::io::Error> for Attempt {
    fn from(error: std::io::Error) -> Self {
        Attempt::Fatal(error.into())
    }
}

impl Attempt {
    /// A retry for a failure that is not worth retrying is fatal.
    fn or_fatal(self) -> Self {
        match self {
            Attempt::Retry {
                failure: Failure::Fatal,
                error,
                ..
            } => Attempt::Fatal(error),
            other => other,
        }
    }
}

/// Download segment `id` to its end, surviving short failures. It stops when
/// the segment is finished, when the scheduler has cut it short and the worker
/// has reached its new end, or when the server pushes back.
async fn run_worker(shared: &Shared, id: usize) -> Result<Ended, Error> {
    let mut failures = 0u32;
    loop {
        let (_, position, end) = shared.scheduler.state(id);
        if position >= end {
            return Ok(Ended::Done);
        }
        match attempt(shared, id, position, end).await {
            Ok(Stop::Reached) => return Ok(Ended::Done),
            Ok(Stop::Retired) => return Ok(Ended::Retired { id }),
            Err(Attempt::Fatal(error)) => return Err(error),
            Err(Attempt::Retry {
                failure,
                retry_after,
                error,
            }) => {
                if shared.scheduler.state(id).1 > position {
                    failures = 0; // it got somewhere: start counting afresh
                }
                if failure == Failure::Throttled {
                    return Ok(Ended::Throttled {
                        id,
                        retry_after,
                        error,
                    });
                }
                failures += 1;
                if failures > shared.policy.max_attempts {
                    return Err(error);
                }
                sleep(shared.policy.backoff(failures, retry_after)).await;
            }
        }
    }
}

/// How a request ended without an error.
enum Stop {
    /// The segment's end was reached.
    Reached,
    /// The worker was asked to stop and did.
    Retired,
}

/// One request for the rest of a segment, from `position` on. Everything
/// received is written, and flushed, before this returns, so the scheduler's
/// position always matches what is in the file.
async fn attempt(shared: &Shared, id: usize, position: u64, end: u64) -> Result<Stop, Attempt> {
    // Waiting for a connection can take a long time on a server that ignores
    // new ones, so a retire request must be able to interrupt it.
    let request = shared
        .client
        .get_range(&shared.url, position, end - 1, shared.etag.as_deref());
    tokio::pin!(request);
    let mut body = loop {
        if shared.should_retire() {
            return Ok(Stop::Retired);
        }
        let woken = shared.wake.notified();
        tokio::select! {
            result = &mut request => break result?,
            () = woken => {}
        }
    };

    let mut file = OpenOptions::new()
        .write(true)
        .open(&shared.part_path)
        .await?;
    file.seek(std::io::SeekFrom::Start(position)).await?;
    // Received bytes are collected here and written in larger pieces. After
    // each write, `reserved_to` is recorded as committed: the segment's bytes
    // before it are really in the file.
    let mut buffer: Vec<u8> = Vec::with_capacity(WRITE_BUFFER);
    let mut reserved_to = position;

    let mut broken = None;
    let mut retired = false;
    'receive: loop {
        let chunk = match body.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                broken = Some(error);
                break;
            }
        };
        // Slow down to the speed limits before taking the bytes.
        shared.limits.acquire(chunk.len()).await;
        if shared.should_retire() {
            retired = true;
            break 'receive;
        }
        let allowed = shared.scheduler.reserve(id, chunk.len() as u64) as usize;
        if allowed == 0 {
            break 'receive;
        }
        buffer.extend_from_slice(&chunk[..allowed]);
        reserved_to += allowed as u64;
        shared
            .downloaded
            .fetch_add(allowed as u64, Ordering::Relaxed);
        if buffer.len() >= WRITE_BUFFER {
            file.write_all(&buffer).await?;
            buffer.clear();
            shared.scheduler.commit(id, reserved_to);
        }
        if allowed < chunk.len() {
            // The segment ends here: the rest belongs to another worker.
            break 'receive;
        }
    }
    if !buffer.is_empty() {
        file.write_all(&buffer).await?;
        shared.scheduler.commit(id, reserved_to);
    }
    file.flush().await?;
    // Close the handle for real before returning (see `fetch_to_part`).
    drop(file.into_std().await);

    if let Some(error) = broken {
        return Err(error.into());
    }
    if retired {
        return Ok(Stop::Retired);
    }
    let (start, now, end) = shared.scheduler.state(id);
    if now != end {
        // The server closed the connection early without saying so.
        return Err(Attempt::Retry {
            failure: Failure::Transient,
            retry_after: None,
            error: Error::Incomplete {
                expected: end - start,
                received: now - start,
            },
        });
    }
    Ok(Stop::Reached)
}

/// "1.4 MiB", "170 KiB" and so on, for notes.
fn human_speed(bytes_per_second: f64) -> String {
    if bytes_per_second >= 1024.0 * 1024.0 {
        format!("{:.1} MiB", bytes_per_second / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KiB", bytes_per_second / 1024.0)
    }
}
