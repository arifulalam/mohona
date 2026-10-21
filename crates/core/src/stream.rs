//! Downloading a stream's segments (HLS or DASH) into one file.
//!
//! Every segment is fetched on its own (several at a time), retried on its
//! own, decrypted if the playlist says so, and saved as a small file in a
//! `<name>.parts` folder next to the result. A segment that is in that
//! folder is done: running the same download again fetches only the rest
//! (resume by segment index). When all are there, they are joined in order,
//! with the init segment in front where one is needed, and the folder is
//! removed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use mohona_media::{ByteRange, Encryption, InitSegment, Segment, SegmentList};
use mohona_net::{Failure, HttpClient, NetError};
use tokio::sync::{mpsc::Sender, Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::limiter::Limits;
use crate::{CancelToken, Error, RateLimit, RetryPolicy};

/// What to download and where to put it.
#[derive(Clone)]
pub struct StreamRequest {
    /// The segments, in order.
    pub segments: SegmentList,
    /// The finished file. Its folder must exist.
    pub dest: PathBuf,
    /// How many segments to fetch at the same time.
    pub connections: usize,
    pub retry: RetryPolicy,
    pub cancel: CancelToken,
    pub limits: Vec<Arc<RateLimit>>,
}

impl StreamRequest {
    /// A request with 8 parallel segments and the default retry rules.
    pub fn new(segments: SegmentList, dest: impl Into<PathBuf>) -> Self {
        Self {
            segments,
            dest: dest.into(),
            connections: 8,
            retry: RetryPolicy::default(),
            cancel: CancelToken::new(),
            limits: Vec::new(),
        }
    }
}

/// Progress of a stream download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    /// Work is about to start; `already` segments were kept from an earlier run.
    Started { segments: usize, already: usize },
    /// A segment is done.
    Progress {
        done: usize,
        total: usize,
        bytes: u64,
    },
    /// The file is complete.
    Finished { path: PathBuf, bytes: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamOutcome {
    pub path: PathBuf,
    pub bytes: u64,
    pub segments: usize,
}

struct Shared {
    client: HttpClient,
    retry: RetryPolicy,
    cancel: CancelToken,
    limits: Limits,
    keys: Mutex<HashMap<String, Vec<u8>>>,
    parts: PathBuf,
    done: AtomicUsize,
    bytes: AtomicU64,
}

/// Fetches all segments and joins them into `request.dest`.
pub async fn download_stream(
    client: &HttpClient,
    request: &StreamRequest,
    events: Option<&Sender<StreamEvent>>,
) -> Result<StreamOutcome, Error> {
    let segments = &request.segments.segments;
    if segments.is_empty() {
        return Err(Error::Stream("the stream has no segments".into()));
    }
    let parts = parts_dir(&request.dest);
    prepare_parts(&parts, &fingerprint(&request.segments))?;

    // Distinct init segments, numbered in order of appearance.
    let mut inits: Vec<InitSegment> = Vec::new();
    for init in segments.iter().filter_map(|s| s.init.as_ref()) {
        if !inits.contains(init) {
            inits.push(init.clone());
        }
    }

    let shared = Arc::new(Shared {
        client: client.clone(),
        retry: request.retry.clone(),
        cancel: request.cancel.clone(),
        limits: Limits(request.limits.clone()),
        keys: Mutex::new(HashMap::new()),
        parts: parts.clone(),
        done: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
    });

    enum Job {
        Init(usize, InitSegment),
        Media(usize, Box<Segment>),
    }
    let mut jobs: Vec<Job> = Vec::new();
    for (i, init) in inits.iter().enumerate() {
        if !parts.join(init_name(i)).is_file() {
            jobs.push(Job::Init(i, init.clone()));
        }
    }
    let mut already = 0;
    for (i, segment) in segments.iter().enumerate() {
        if parts.join(part_name(i)).is_file() {
            already += 1;
        } else {
            jobs.push(Job::Media(i, Box::new(segment.clone())));
        }
    }
    shared.done.store(already, Ordering::Relaxed);
    send(
        events,
        StreamEvent::Started {
            segments: segments.len(),
            already,
        },
    )
    .await;

    let gate = Arc::new(Semaphore::new(request.connections.max(1)));
    let mut tasks: JoinSet<Result<(), Error>> = JoinSet::new();
    let total = segments.len();
    let mut outcome: Result<(), Error> = Ok(());
    let mut jobs = jobs.into_iter();
    'feed: loop {
        // Keep the pool full; collect finished tasks as they come.
        let permit = tokio::select! {
            p = gate.clone().acquire_owned() => p.expect("the semaphore is never closed"),
            Some(done) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(error) = flatten(done) {
                    outcome = Err(error);
                    break 'feed;
                }
                continue 'feed;
            }
            () = request.cancel.cancelled() => { outcome = Err(Error::Cancelled); break 'feed; }
        };
        let Some(job) = jobs.next() else { break };
        let shared = Arc::clone(&shared);
        let events = events.cloned();
        tasks.spawn(async move {
            let _permit = permit;
            match job {
                Job::Init(i, init) => {
                    let data = fetch(&shared, init.url.as_ref(), init.range).await?;
                    store(&shared.parts, &init_name(i), &data).await
                }
                Job::Media(i, segment) => {
                    let mut data = fetch(&shared, segment.url.as_ref(), segment.range).await?;
                    if let Some(Encryption::Aes128 { key_url, .. }) = &segment.encryption {
                        let key = key_for(&shared, key_url.as_ref()).await?;
                        let iv = segment.iv().expect("an encrypted segment has an IV");
                        data = decrypt(&key, &iv, &data, i)?;
                    }
                    store(&shared.parts, &part_name(i), &data).await?;
                    let done = shared.done.fetch_add(1, Ordering::Relaxed) + 1;
                    let bytes = shared.bytes.fetch_add(data.len() as u64, Ordering::Relaxed)
                        + data.len() as u64;
                    send(
                        events.as_ref(),
                        StreamEvent::Progress { done, total, bytes },
                    )
                    .await;
                    Ok(())
                }
            }
        });
    }
    if outcome.is_ok() {
        loop {
            tokio::select! {
                done = tasks.join_next() => match done {
                    None => break,
                    Some(done) => if let Err(error) = flatten(done) {
                        outcome = Err(error);
                        break;
                    },
                },
                () = request.cancel.cancelled() => { outcome = Err(Error::Cancelled); break; }
            }
        }
    }
    if let Err(error) = outcome {
        // Stop everything and let the file handles close before returning.
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        return Err(error);
    }

    // Join the pieces.
    let dest = request.dest.clone();
    let layout: Vec<(Option<usize>, usize)> = segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (
                s.init
                    .as_ref()
                    .and_then(|init| inits.iter().position(|x| x == init)),
                i,
            )
        })
        .collect();
    let parts_dir = parts.clone();
    let bytes = tokio::task::spawn_blocking(move || assemble(&dest, &parts_dir, &layout))
        .await
        .map_err(std::io::Error::other)??;
    send(
        events,
        StreamEvent::Finished {
            path: request.dest.clone(),
            bytes,
        },
    )
    .await;
    Ok(StreamOutcome {
        path: request.dest.clone(),
        bytes,
        segments: total,
    })
}

fn flatten(done: Result<Result<(), Error>, tokio::task::JoinError>) -> Result<(), Error> {
    match done {
        Ok(result) => result,
        Err(error) => Err(Error::Io(std::io::Error::other(error))),
    }
}

async fn send(events: Option<&Sender<StreamEvent>>, event: StreamEvent) {
    if let Some(events) = events {
        let _ = events.send(event).await;
    }
}

/// `movie.ts` -> `movie.ts.parts`.
pub fn parts_dir(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".parts");
    dest.with_file_name(name)
}

fn part_name(i: usize) -> String {
    format!("{i:06}")
}

fn init_name(i: usize) -> String {
    format!("init-{i}")
}

/// Identifies the segment list, so pieces of a different playlist are not
/// mixed in.
fn fingerprint(list: &SegmentList) -> String {
    let first = &list.segments[0];
    let last = &list.segments[list.segments.len() - 1];
    format!(
        "{} {} {} {:?} {:?}",
        list.segments.len(),
        first.url,
        last.url,
        first.range,
        last.range
    )
}

fn prepare_parts(parts: &Path, fingerprint: &str) -> Result<(), Error> {
    let marker = parts.join("plan");
    if parts.is_dir() && std::fs::read_to_string(&marker).ok().as_deref() != Some(fingerprint) {
        std::fs::remove_dir_all(parts)?;
    }
    std::fs::create_dir_all(parts)?;
    if !marker.is_file() {
        std::fs::write(&marker, fingerprint)?;
    }
    Ok(())
}

/// Writes a piece so that it either exists completely or not at all.
async fn store(parts: &Path, name: &str, data: &[u8]) -> Result<(), Error> {
    let tmp = parts.join(format!("{name}.tmp"));
    let target = parts.join(name);
    let data = data.to_vec();
    tokio::task::spawn_blocking(move || {
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &target)
    })
    .await
    .map_err(std::io::Error::other)??;
    Ok(())
}

fn assemble(dest: &Path, parts: &Path, layout: &[(Option<usize>, usize)]) -> Result<u64, Error> {
    use std::io::Write;
    let tmp = {
        let mut name = dest.file_name().unwrap_or_default().to_os_string();
        name.push(".tmp");
        dest.with_file_name(name)
    };
    let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    let mut written = 0u64;
    let mut last_init: Option<usize> = None;
    for (init, index) in layout {
        if let Some(init) = init {
            if last_init != Some(*init) {
                let data = std::fs::read(parts.join(init_name(*init)))?;
                out.write_all(&data)?;
                written += data.len() as u64;
                last_init = Some(*init);
            }
        }
        let data = std::fs::read(parts.join(part_name(*index)))?;
        out.write_all(&data)?;
        written += data.len() as u64;
    }
    out.flush()?;
    out.into_inner()
        .map_err(std::io::Error::other)?
        .sync_all()?;
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    std::fs::rename(&tmp, dest)?;
    let _ = std::fs::remove_dir_all(parts);
    Ok(written)
}

/// One piece, with retries.
async fn fetch(shared: &Shared, url: &str, range: Option<ByteRange>) -> Result<Vec<u8>, Error> {
    let mut failures = 0u32;
    let mut throttles = 0u32;
    loop {
        if shared.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let error = match fetch_once(shared, url, range).await {
            Ok(data) => return Ok(data),
            Err(error) => error,
        };
        let wait = match error.failure() {
            Failure::Fatal => return Err(error.into()),
            Failure::Transient => {
                failures += 1;
                if failures >= shared.retry.max_attempts {
                    return Err(error.into());
                }
                shared.retry.backoff(failures, error.retry_after())
            }
            Failure::Throttled => {
                throttles += 1;
                if throttles >= shared.retry.max_throttles {
                    return Err(error.into());
                }
                shared.retry.backoff(throttles, error.retry_after())
            }
        };
        tokio::select! {
            () = sleep(wait) => {}
            () = shared.cancel.cancelled() => return Err(Error::Cancelled),
        }
    }
}

async fn fetch_once(
    shared: &Shared,
    url: &str,
    range: Option<ByteRange>,
) -> Result<Vec<u8>, NetError> {
    let mut body = match range {
        Some(r) => {
            shared
                .client
                .get_range(url, r.offset, r.offset + r.length.max(1) - 1, None)
                .await?
        }
        None => shared.client.get(url).await?,
    };
    let mut data = Vec::new();
    while let Some(chunk) = body.chunk().await? {
        shared.limits.acquire(chunk.len()).await;
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

async fn key_for(shared: &Shared, url: &str) -> Result<Vec<u8>, Error> {
    // One at a time, so a key shared by many segments is fetched once.
    let mut keys = shared.keys.lock().await;
    if let Some(key) = keys.get(url) {
        return Ok(key.clone());
    }
    let key = fetch(shared, url, None).await?;
    if key.len() != 16 {
        return Err(Error::Stream(format!(
            "the encryption key at {url} is {} bytes, not 16 (the server may require a login or cookie)",
            key.len()
        )));
    }
    keys.insert(url.to_string(), key.clone());
    Ok(key)
}

fn decrypt(key: &[u8], iv: &[u8; 16], data: &[u8], index: usize) -> Result<Vec<u8>, Error> {
    cbc::Decryptor::<aes::Aes128>::new_from_slices(key, iv)
        .map_err(|_| Error::Stream("the encryption key has the wrong size".into()))?
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| {
            Error::Stream(format!(
                "segment {index} could not be decrypted (wrong key or damaged data)"
            ))
        })
}
