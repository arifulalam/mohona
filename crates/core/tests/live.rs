use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use mohona_core::{record_live, Error, LiveEnd, LiveEvent, LiveRequest, RetryPolicy};
use mohona_net::{ClientConfig, HttpClient};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

/// Answers every request with `handler(path, how many playlist requests so far)`.
type Handler = Arc<dyn Fn(&str, usize) -> Option<Vec<u8>> + Send + Sync>;

async fn serve(handler: Handler) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let playlists = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (handler, playlists) = (handler.clone(), playlists.clone());
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 2048];
                while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buffer).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let k = if path.ends_with(".m3u8") {
                    playlists.fetch_add(1, Ordering::SeqCst)
                } else {
                    0
                };
                let (status, body) = match handler(&path, k) {
                    Some(body) => (200, body),
                    None => (404, Vec::new()),
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    addr
}

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap()
}

fn piece(i: u64) -> Vec<u8> {
    (0..100).map(|j| ((i * 13 + j) % 251) as u8).collect()
}

fn playlist(first: u64, last: u64, end: bool) -> Vec<u8> {
    let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:{first}\n");
    for i in first..=last {
        text.push_str(&format!("#EXTINF:4,\nseg{i}.ts\n"));
    }
    if end {
        text.push_str("#EXT-X-ENDLIST\n");
    }
    text.into_bytes()
}

fn segment(path: &str) -> Option<Vec<u8>> {
    let n = path
        .strip_prefix("/seg")?
        .strip_suffix(".ts")?
        .parse()
        .ok()?;
    Some(piece(n))
}

fn request(addr: SocketAddr, dir: &std::path::Path) -> LiveRequest {
    let mut r = LiveRequest::new(
        Url::parse(&format!("http://{addr}/live.m3u8")).unwrap(),
        dir.join("rec.ts"),
    );
    r.reload = Some(Duration::from_millis(20));
    r.retry = RetryPolicy {
        base_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(20),
        max_attempts: 3,
        ..RetryPolicy::default()
    };
    r
}

fn joined(ids: &[u64]) -> Vec<u8> {
    ids.iter().flat_map(|i| piece(*i)).collect()
}

/// Request k sees a window of three segments that slides by one; the
/// stream ends with segment 7.
fn sliding() -> Handler {
    Arc::new(|path, k| {
        if path == "/live.m3u8" {
            let newest = (k as u64 + 1).min(7);
            Some(playlist(newest.saturating_sub(2), newest, newest >= 7))
        } else {
            segment(path)
        }
    })
}

#[tokio::test]
async fn records_a_sliding_window_until_the_stream_ends() {
    let addr = serve(sliding()).await;
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let outcome = record_live(&client(), &request(addr, dir.path()), Some(&tx))
        .await
        .unwrap();
    drop(tx);
    assert_eq!(outcome.ended, LiveEnd::StreamEnded);
    assert_eq!(outcome.segments, 8);
    assert_eq!(outcome.seconds, 32.0);
    assert_eq!(outcome.bytes, 800);
    // Every segment exactly once, in order, although the windows overlap.
    assert_eq!(
        std::fs::read(dir.path().join("rec.ts")).unwrap(),
        joined(&[0, 1, 2, 3, 4, 5, 6, 7])
    );
    let mut events = Vec::new();
    while let Some(e) = rx.recv().await {
        events.push(e);
    }
    assert_eq!(events.first(), Some(&LiveEvent::Started));
    assert!(!events.iter().any(|e| matches!(e, LiveEvent::Missed { .. })));
}

#[tokio::test]
async fn segments_that_slipped_out_of_the_window_are_reported() {
    let addr = serve(Arc::new(|path, k| {
        if path == "/live.m3u8" {
            Some(if k == 0 {
                playlist(0, 1, false)
            } else {
                playlist(4, 5, true)
            })
        } else {
            segment(path)
        }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let outcome = record_live(&client(), &request(addr, dir.path()), Some(&tx))
        .await
        .unwrap();
    drop(tx);
    assert_eq!(outcome.ended, LiveEnd::StreamEnded);
    assert_eq!(
        std::fs::read(dir.path().join("rec.ts")).unwrap(),
        joined(&[0, 1, 4, 5])
    );
    let mut missed = 0;
    while let Some(e) = rx.recv().await {
        if let LiveEvent::Missed { count } = e {
            missed += count;
        }
    }
    assert_eq!(missed, 2);
}

#[tokio::test]
async fn a_time_limit_stops_the_recording() {
    // A stream that never ends.
    let addr = serve(Arc::new(|path, k| {
        if path == "/live.m3u8" {
            let newest = k as u64 + 1;
            Some(playlist(newest.saturating_sub(2), newest, false))
        } else {
            segment(path)
        }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(addr, dir.path());
    r.max_seconds = Some(12.0);
    let outcome = record_live(&client(), &r, None).await.unwrap();
    assert_eq!(outcome.ended, LiveEnd::TimeLimit);
    assert_eq!(outcome.segments, 3);
    assert_eq!(
        std::fs::read(dir.path().join("rec.ts")).unwrap(),
        joined(&[0, 1, 2])
    );
}

#[tokio::test]
async fn stopping_keeps_a_playable_file() {
    let addr = serve(Arc::new(|path, k| {
        if path == "/live.m3u8" {
            let newest = k as u64 + 1;
            Some(playlist(newest.saturating_sub(2), newest, false))
        } else {
            segment(path)
        }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(addr, dir.path());
    let cancel = r.cancel.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let client = client();
    let worker = {
        let r = r.clone();
        tokio::spawn(async move { record_live(&client, &r, Some(&tx)).await })
    };
    while let Some(event) = rx.recv().await {
        if matches!(event, LiveEvent::Segment { count: 4, .. }) {
            cancel.cancel();
            break;
        }
    }
    while rx.recv().await.is_some() {}
    let outcome = worker.await.unwrap().unwrap();
    assert_eq!(outcome.ended, LiveEnd::Stopped);
    assert!(outcome.segments >= 4);
    let data = std::fs::read(dir.path().join("rec.ts")).unwrap();
    assert_eq!(data.len() as u64, outcome.bytes);
    let ids: Vec<u64> = (0..outcome.segments as u64).collect();
    assert_eq!(data, joined(&ids));
}

#[tokio::test]
async fn a_playlist_that_disappears_ends_the_recording_but_keeps_it() {
    let addr = serve(Arc::new(|path, k| {
        if path == "/live.m3u8" {
            (k < 2).then(|| playlist(k as u64, k as u64 + 1, false))
        } else {
            segment(path)
        }
    }))
    .await;
    let dir = tempfile::tempdir().unwrap();
    let outcome = record_live(&client(), &request(addr, dir.path()), None)
        .await
        .unwrap();
    assert_eq!(outcome.ended, LiveEnd::PlaylistGone);
    assert_eq!(outcome.segments, 3);
    assert_eq!(
        std::fs::read(dir.path().join("rec.ts")).unwrap(),
        joined(&[0, 1, 2])
    );
}

#[tokio::test]
async fn a_playlist_that_is_missing_from_the_start_is_an_error() {
    let addr = serve(Arc::new(|_, _| None)).await;
    let dir = tempfile::tempdir().unwrap();
    let error = record_live(&client(), &request(addr, dir.path()), None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("404"), "{error}");
    let master = serve(Arc::new(|path, _| {
        (path == "/live.m3u8").then(|| b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n".to_vec())
    }))
    .await;
    let error = record_live(&client(), &request(master, dir.path()), None)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Stream(_)));
    assert!(error.to_string().contains("master playlist"), "{error}");
}

#[tokio::test]
async fn encrypted_live_segments_with_an_init_part_are_handled() {
    let key = *b"0123456789abcdef";
    let handler: Handler = Arc::new(move |path, k| match path {
        "/live.m3u8" => {
            let newest = (k as u64 + 1).min(3);
            let mut text = String::from(
                "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n\
                 #EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n",
            );
            for i in 0..=newest {
                text.push_str(&format!("#EXTINF:4,\nseg{i}.ts\n"));
            }
            if newest >= 3 {
                text.push_str("#EXT-X-ENDLIST\n");
            }
            Some(text.into_bytes())
        }
        "/key.bin" => Some(key.to_vec()),
        "/init.mp4" => Some(b"INIT".to_vec()),
        other => {
            let n: u64 = other
                .strip_prefix("/seg")?
                .strip_suffix(".ts")?
                .parse()
                .ok()?;
            let mut iv = [0u8; 16];
            iv[8..].copy_from_slice(&n.to_be_bytes());
            Some(
                cbc::Encryptor::<aes::Aes128>::new((&key).into(), (&iv).into())
                    .encrypt_padded_vec_mut::<Pkcs7>(&piece(n)),
            )
        }
    });
    let addr = serve(handler).await;
    let dir = tempfile::tempdir().unwrap();
    let outcome = record_live(&client(), &request(addr, dir.path()), None)
        .await
        .unwrap();
    assert_eq!(outcome.ended, LiveEnd::StreamEnded);
    // The init part once, at the front.
    let mut want = b"INIT".to_vec();
    want.extend(joined(&[0, 1, 2, 3]));
    assert_eq!(std::fs::read(dir.path().join("rec.ts")).unwrap(), want);
}
