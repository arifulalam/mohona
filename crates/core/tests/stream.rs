use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use mohona_core::{download_stream, Error, RetryPolicy, StreamEvent, StreamRequest};
use mohona_media::hls::{parse, Playlist};
use mohona_media::SegmentList;
use mohona_net::{ClientConfig, HttpClient};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

#[derive(Default)]
struct State {
    files: Mutex<HashMap<String, Vec<u8>>>,
    /// path -> (status, how many more times to answer with it; 0 = always)
    fail: Mutex<HashMap<String, (u16, u32)>>,
    hits: Mutex<HashMap<String, u32>>,
}

impl State {
    fn put(&self, path: &str, data: Vec<u8>) {
        self.files.lock().unwrap().insert(path.to_string(), data);
    }
    fn hits(&self, path: &str) -> u32 {
        *self.hits.lock().unwrap().get(path).unwrap_or(&0)
    }
}

async fn serve() -> (SocketAddr, Arc<State>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(State::default());
    let shared = state.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let state = shared.clone();
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
                *state.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
                let range = head.lines().find_map(|l| {
                    let l = l.to_ascii_lowercase();
                    let v = l.strip_prefix("range: bytes=")?.to_string();
                    let (a, b) = v.split_once('-')?;
                    Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                });
                let forced = {
                    let mut fail = state.fail.lock().unwrap();
                    match fail.get_mut(&path) {
                        Some((status, left)) if *left != 1 => {
                            let status = *status;
                            if *left > 1 {
                                *left -= 1;
                            }
                            Some(status)
                        }
                        Some((status, _)) => {
                            let status = *status;
                            fail.remove(&path);
                            Some(status)
                        }
                        None => None,
                    }
                };
                let file = state.files.lock().unwrap().get(&path).cloned();
                let total = file.as_ref().map_or(0, Vec::len);
                let (status, body) = match (forced, file, range) {
                    (Some(status), _, _) => (status, Vec::new()),
                    (None, None, _) => (404, Vec::new()),
                    (None, Some(f), None) => (200, f),
                    (None, Some(f), Some((a, b))) => (206, f[a..=b.min(f.len() - 1)].to_vec()),
                };
                let content_range = match (status, range) {
                    (206, Some((a, _))) => {
                        format!(
                            "Content-Range: bytes {a}-{}/{total}\r\n",
                            a + body.len() - 1
                        )
                    }
                    _ => String::new(),
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\n{content_range}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (addr, state)
}

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap()
}

fn fast() -> RetryPolicy {
    RetryPolicy {
        base_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(20),
        ..RetryPolicy::default()
    }
}

fn piece(i: usize) -> Vec<u8> {
    (0..1000).map(|j| ((i * 7 + j) % 251) as u8).collect()
}

fn playlist(addr: SocketAddr, text: &str) -> SegmentList {
    let base = Url::parse(&format!("http://{addr}/v/index.m3u8")).unwrap();
    match parse(text, &base).unwrap() {
        Playlist::Media(list) => list,
        Playlist::Master { .. } => panic!("expected a media playlist"),
    }
}

fn plain_playlist(count: usize) -> String {
    let mut text = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n");
    for i in 0..count {
        text.push_str(&format!("#EXTINF:4,\nseg{i}.ts\n"));
    }
    text + "#EXT-X-ENDLIST\n"
}

fn expected(count: usize) -> Vec<u8> {
    (0..count).flat_map(piece).collect()
}

#[tokio::test]
async fn segments_are_joined_in_order() {
    let (addr, state) = serve().await;
    for i in 0..20 {
        state.put(&format!("/v/seg{i}.ts"), piece(i));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut request = StreamRequest::new(
        playlist(addr, &plain_playlist(20)),
        dir.path().join("out.ts"),
    );
    request.connections = 6;
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let outcome = download_stream(&client(), &request, Some(&tx))
        .await
        .unwrap();
    drop(tx);
    assert_eq!(outcome.segments, 20);
    assert_eq!(outcome.bytes, 20_000);
    assert_eq!(
        std::fs::read(dir.path().join("out.ts")).unwrap(),
        expected(20)
    );
    // Only the finished file is left.
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["out.ts"]);
    let mut events = Vec::new();
    while let Some(e) = rx.recv().await {
        events.push(e);
    }
    assert_eq!(
        events.first(),
        Some(&StreamEvent::Started {
            segments: 20,
            already: 0
        })
    );
    assert!(matches!(
        events.last(),
        Some(StreamEvent::Finished { bytes: 20_000, .. })
    ));
    let done: Vec<usize> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Progress { done, .. } => Some(*done),
            _ => None,
        })
        .collect();
    assert_eq!(done.len(), 20);
    assert_eq!(done.iter().max(), Some(&20));
}

#[tokio::test]
async fn a_hiccup_on_one_segment_is_retried() {
    let (addr, state) = serve().await;
    for i in 0..5 {
        state.put(&format!("/v/seg{i}.ts"), piece(i));
    }
    state
        .fail
        .lock()
        .unwrap()
        .insert("/v/seg2.ts".into(), (503, 2));
    state
        .fail
        .lock()
        .unwrap()
        .insert("/v/seg3.ts".into(), (500, 1));
    let dir = tempfile::tempdir().unwrap();
    let mut request =
        StreamRequest::new(playlist(addr, &plain_playlist(5)), dir.path().join("o.ts"));
    request.retry = fast();
    download_stream(&client(), &request, None).await.unwrap();
    assert_eq!(std::fs::read(dir.path().join("o.ts")).unwrap(), expected(5));
    assert_eq!(state.hits("/v/seg2.ts"), 3);
    assert_eq!(state.hits("/v/seg3.ts"), 2);
    assert_eq!(state.hits("/v/seg0.ts"), 1);
}

#[tokio::test]
async fn byte_range_segments_are_cut_from_one_file() {
    let (addr, state) = serve().await;
    let whole: Vec<u8> = (0..3000).map(|i| (i % 253) as u8).collect();
    state.put("/v/all.ts", whole.clone());
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
        #EXTINF:4,\n#EXT-X-BYTERANGE:1000@0\nall.ts\n\
        #EXTINF:4,\n#EXT-X-BYTERANGE:1000\nall.ts\n\
        #EXTINF:4,\n#EXT-X-BYTERANGE:500\nall.ts\n#EXT-X-ENDLIST\n";
    let dir = tempfile::tempdir().unwrap();
    let request = StreamRequest::new(playlist(addr, text), dir.path().join("o.ts"));
    let outcome = download_stream(&client(), &request, None).await.unwrap();
    assert_eq!(outcome.bytes, 2500);
    assert_eq!(
        std::fs::read(dir.path().join("o.ts")).unwrap(),
        whole[..2500]
    );
}

fn encrypt(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    cbc::Encryptor::<aes::Aes128>::new(key.into(), iv.into()).encrypt_padded_vec_mut::<Pkcs7>(data)
}

#[tokio::test]
async fn aes_128_segments_are_decrypted_and_init_comes_first() {
    let (addr, state) = serve().await;
    let key = *b"0123456789abcdef";
    state.put("/v/key.bin", key.to_vec());
    state.put("/v/init.mp4", b"INIT".to_vec());
    // Segment numbers start at 10 and no IV is given: the number is the IV.
    for i in 0..4usize {
        let mut iv = [0u8; 16];
        iv[8..].copy_from_slice(&((10 + i) as u64).to_be_bytes());
        state.put(&format!("/v/s{i}.m4s"), encrypt(&key, &iv, &piece(i)));
    }
    let mut text = String::from(
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:10\n\
         #EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n",
    );
    for i in 0..4 {
        text.push_str(&format!("#EXTINF:4,\ns{i}.m4s\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    let dir = tempfile::tempdir().unwrap();
    let request = StreamRequest::new(playlist(addr, &text), dir.path().join("o.mp4"));
    download_stream(&client(), &request, None).await.unwrap();
    let mut want = b"INIT".to_vec();
    want.extend(expected(4));
    assert_eq!(std::fs::read(dir.path().join("o.mp4")).unwrap(), want);
    // The key and the init segment are each fetched once.
    assert_eq!(state.hits("/v/key.bin"), 1);
    assert_eq!(state.hits("/v/init.mp4"), 1);
}

#[tokio::test]
async fn a_wrong_key_gets_a_clear_error() {
    let (addr, state) = serve().await;
    state.put("/v/key.bin", b"short".to_vec());
    state.put("/v/s0.ts", vec![0; 32]);
    let text = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:4,\ns0.ts\n#EXT-X-ENDLIST\n";
    let dir = tempfile::tempdir().unwrap();
    let request = StreamRequest::new(playlist(addr, text), dir.path().join("o.ts"));
    let err = download_stream(&client(), &request, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not 16"), "{err}");

    // A good-sized but wrong key fails to decrypt.
    state.put("/v/key.bin", b"0123456789abcdef".to_vec());
    state.put("/v/s0.ts", vec![7; 32]);
    let err = download_stream(&client(), &request, None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Stream(_)));
    assert!(err.to_string().contains("could not be decrypted"), "{err}");
}

#[tokio::test]
async fn a_failed_run_keeps_finished_segments_and_resumes() {
    let (addr, state) = serve().await;
    for i in 0..10 {
        state.put(&format!("/v/seg{i}.ts"), piece(i));
    }
    // Segment 6 is gone for now (404 is final, so the first run fails).
    state
        .fail
        .lock()
        .unwrap()
        .insert("/v/seg6.ts".into(), (404, 0));
    let dir = tempfile::tempdir().unwrap();
    let mut request =
        StreamRequest::new(playlist(addr, &plain_playlist(10)), dir.path().join("o.ts"));
    request.connections = 2;
    request.retry = fast();
    let err = download_stream(&client(), &request, None)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("404") || err.to_string().to_lowercase().contains("not"),
        "{err}"
    );
    assert!(!dir.path().join("o.ts").exists());
    let parts = dir.path().join("o.ts.parts");
    assert!(parts.is_dir());
    // A piece being written when the run stopped may still land a moment
    // later (the write finishes on its own thread), so let things settle.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let is_piece = |n: &str| n.len() == 6 && n.chars().all(|c| c.is_ascii_digit());
    let kept_pieces: Vec<usize> = std::fs::read_dir(&parts)
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().to_string_lossy().to_string();
            is_piece(&n).then(|| n.parse().unwrap())
        })
        .collect();
    let kept = kept_pieces.len();
    assert!(kept >= 1, "some segments should have been kept");

    state.fail.lock().unwrap().clear();
    let hits_before: Vec<u32> = (0..10)
        .map(|i| state.hits(&format!("/v/seg{i}.ts")))
        .collect();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    download_stream(&client(), &request, Some(&tx))
        .await
        .unwrap();
    drop(tx);
    assert_eq!(
        std::fs::read(dir.path().join("o.ts")).unwrap(),
        expected(10)
    );
    let Some(StreamEvent::Started { already, .. }) = rx.recv().await else {
        panic!("expected a Started event")
    };
    assert_eq!(already, kept);
    // Segments that were kept were not fetched again; all others were.
    for (i, before) in hits_before.iter().enumerate() {
        let again = state.hits(&format!("/v/seg{i}.ts")) > *before;
        assert_eq!(again, !kept_pieces.contains(&i), "segment {i}");
    }
    assert!(!parts.exists());
}

#[tokio::test]
async fn a_different_playlist_does_not_reuse_old_pieces() {
    let (addr, state) = serve().await;
    for i in 0..3 {
        state.put(&format!("/v/seg{i}.ts"), piece(i));
    }
    let dir = tempfile::tempdir().unwrap();
    let parts = dir.path().join("o.ts.parts");
    std::fs::create_dir_all(&parts).unwrap();
    std::fs::write(parts.join("plan"), "something else").unwrap();
    std::fs::write(parts.join("000000"), b"STALE").unwrap();
    let request = StreamRequest::new(playlist(addr, &plain_playlist(3)), dir.path().join("o.ts"));
    download_stream(&client(), &request, None).await.unwrap();
    assert_eq!(std::fs::read(dir.path().join("o.ts")).unwrap(), expected(3));
}

#[tokio::test]
async fn stopping_keeps_what_is_done() {
    let (addr, state) = serve().await;
    for i in 0..4 {
        state.put(&format!("/v/seg{i}.ts"), piece(i));
    }
    let dir = tempfile::tempdir().unwrap();
    let request = StreamRequest::new(playlist(addr, &plain_playlist(4)), dir.path().join("o.ts"));
    request.cancel.cancel();
    let err = download_stream(&client(), &request, None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Cancelled));
    assert!(!dir.path().join("o.ts").exists());
}
