//! Recording a live HLS stream.
//!
//! A live playlist has no end: it lists the last few segments and changes
//! every few seconds. [`record_live`] reloads it, fetches each new segment
//! once (a few at a time), and appends them in order to one file, until the
//! stream says it has ended, the person stops it, or a time limit is reached.
//! The file is playable at every moment, so stopping never loses anything.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mohona_media::hls::{self, Playlist};
use mohona_media::{Encryption, InitSegment, Segment};
use mohona_net::HttpClient;
use tokio::sync::mpsc::Sender;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::sleep;
use url::Url;

use crate::stream::{decrypt, fetch, key_for, Shared};
use crate::{CancelToken, Error, RateLimit, RetryPolicy};

#[derive(Clone)]
pub struct LiveRequest {
    /// The address of the live media playlist.
    pub playlist: Url,
    /// The file to write. It is replaced if it exists.
    pub dest: PathBuf,
    /// Segments fetched at once (a live stream rarely needs more than 4).
    pub connections: usize,
    pub retry: RetryPolicy,
    pub cancel: CancelToken,
    pub limits: Vec<Arc<RateLimit>>,
    /// Stop after this many seconds of recorded video.
    pub max_seconds: Option<f64>,
    /// How long to wait between playlist reloads. Default: half the
    /// segment length, between 0.5 and 5 seconds.
    pub reload: Option<Duration>,
}

impl LiveRequest {
    pub fn new(playlist: Url, dest: impl Into<PathBuf>) -> Self {
        Self {
            playlist,
            dest: dest.into(),
            connections: 4,
            retry: RetryPolicy::default(),
            cancel: CancelToken::new(),
            limits: Vec::new(),
            max_seconds: None,
            reload: None,
        }
    }
}

/// Why a recording ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveEnd {
    /// The playlist said the stream is over.
    StreamEnded,
    /// The person stopped it.
    Stopped,
    /// The time limit was reached.
    TimeLimit,
    /// The playlist could not be fetched any more (usually the stream is over
    /// without saying so).
    PlaylistGone,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LiveEvent {
    Started,
    /// A segment was added: totals so far.
    Segment {
        count: usize,
        seconds: f64,
        bytes: u64,
    },
    /// Segments the stream dropped before they could be fetched.
    Missed {
        count: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveOutcome {
    pub path: PathBuf,
    pub bytes: u64,
    pub segments: usize,
    pub seconds: f64,
    pub ended: LiveEnd,
}

async fn send(events: Option<&Sender<LiveEvent>>, event: LiveEvent) {
    if let Some(events) = events {
        let _ = events.send(event).await;
    }
}

/// Records the stream into `request.dest`.
pub async fn record_live(
    client: &HttpClient,
    request: &LiveRequest,
    events: Option<&Sender<LiveEvent>>,
) -> Result<LiveOutcome, Error> {
    let shared = Shared::for_live(client, &request.retry, &request.cancel, &request.limits);
    let mut out = std::io::BufWriter::new(std::fs::File::create(&request.dest)?);
    send(events, LiveEvent::Started).await;

    let mut next: Option<u64> = None;
    let mut last_init: Option<InitSegment> = None;
    let (mut count, mut seconds, mut bytes) = (0usize, 0f64, 0u64);
    let gate = Arc::new(Semaphore::new(request.connections.max(1)));

    let ended = loop {
        if request.cancel.is_cancelled() {
            break LiveEnd::Stopped;
        }
        let text = match fetch(&shared, request.playlist.as_str(), None).await {
            Ok(data) => String::from_utf8_lossy(&data).into_owned(),
            Err(Error::Cancelled) => break LiveEnd::Stopped,
            Err(error) if count == 0 => return Err(error),
            Err(_) => break LiveEnd::PlaylistGone,
        };
        let list = match hls::parse(&text, &request.playlist) {
            Ok(Playlist::Media(list)) => list,
            Ok(Playlist::Master { .. }) => {
                return Err(Error::Stream(
                    "this is a master playlist; give the address of one quality's playlist".into(),
                ))
            }
            Err(error) if count == 0 => return Err(Error::Stream(error.to_string())),
            Err(_) => break LiveEnd::PlaylistGone,
        };

        let fresh: Vec<Segment> = list
            .segments
            .iter()
            .filter(|s| s.sequence >= next.unwrap_or(0))
            .cloned()
            .collect();
        if let (Some(expected), Some(first)) = (next, fresh.first()) {
            if first.sequence > expected {
                send(
                    events,
                    LiveEvent::Missed {
                        count: first.sequence - expected,
                    },
                )
                .await;
            }
        }

        // Fetch the new segments a few at a time, but write them in order.
        let mut tasks: JoinSet<(usize, Result<Vec<u8>, Error>)> = JoinSet::new();
        for (index, segment) in fresh.iter().cloned().enumerate() {
            let (shared, gate) = (Arc::clone(&shared), Arc::clone(&gate));
            tasks.spawn(async move {
                let _permit = gate.acquire_owned().await;
                (index, fetch_segment(&shared, &segment).await)
            });
        }
        let mut results: Vec<Option<Result<Vec<u8>, Error>>> =
            (0..fresh.len()).map(|_| None).collect();
        let mut stopped = false;
        while !tasks.is_empty() {
            tokio::select! {
                done = tasks.join_next() => {
                    if let Some(Ok((index, result))) = done {
                        results[index] = Some(result);
                    }
                }
                () = request.cancel.cancelled() => {
                    tasks.abort_all();
                    stopped = true;
                    break;
                }
            }
        }
        let mut limit_reached = false;
        for (segment, result) in fresh.iter().zip(results) {
            if stopped {
                break;
            }
            match result {
                Some(Ok(data)) => {
                    if segment.init.is_some() && segment.init != last_init {
                        if let Some(init) = &segment.init {
                            let data = fetch(&shared, init.url.as_str(), init.range).await?;
                            out.write_all(&data)?;
                            bytes += data.len() as u64;
                        }
                        last_init = segment.init.clone();
                    }
                    out.write_all(&data)?;
                    bytes += data.len() as u64;
                    seconds += segment.duration;
                    count += 1;
                    next = Some(segment.sequence + 1);
                    send(
                        events,
                        LiveEvent::Segment {
                            count,
                            seconds,
                            bytes,
                        },
                    )
                    .await;
                    if request.max_seconds.is_some_and(|m| seconds >= m) {
                        limit_reached = true;
                        break;
                    }
                }
                Some(Err(Error::Cancelled)) | None => {
                    stopped = true;
                    break;
                }
                Some(Err(error)) => {
                    // A segment that is gone for good is a gap, not the end.
                    if matches!(error, Error::Stream(_)) {
                        return Err(error);
                    }
                    next = Some(segment.sequence + 1);
                    send(events, LiveEvent::Missed { count: 1 }).await;
                }
            }
        }
        out.flush()?;
        if stopped {
            break LiveEnd::Stopped;
        }
        if limit_reached {
            break LiveEnd::TimeLimit;
        }
        if list.ended {
            break LiveEnd::StreamEnded;
        }
        let wait = request.reload.unwrap_or_else(|| {
            Duration::from_secs_f64((list.target_duration / 2.0).clamp(0.5, 5.0))
        });
        tokio::select! {
            () = sleep(wait) => {}
            () = request.cancel.cancelled() => break LiveEnd::Stopped,
        }
    };

    out.flush()?;
    out.into_inner()
        .map_err(std::io::Error::other)?
        .sync_all()?;
    Ok(LiveOutcome {
        path: request.dest.clone(),
        bytes,
        segments: count,
        seconds,
        ended,
    })
}

async fn fetch_segment(shared: &Shared, segment: &Segment) -> Result<Vec<u8>, Error> {
    let mut data = fetch(shared, segment.url.as_str(), segment.range).await?;
    if let Some(Encryption::Aes128 { key_url, .. }) = &segment.encryption {
        let key = key_for(shared, key_url.as_str()).await?;
        let iv = segment.iv().expect("an encrypted segment has an IV");
        data = decrypt(&key, &iv, &data, segment.sequence as usize)?;
    }
    Ok(data)
}
