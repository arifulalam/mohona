//! End-to-end download tests against the local test server.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use mohona_core::{
    download, partial_progress, CancelToken, DownloadRequest, Error, Event, RetryPolicy,
};
use mohona_net::{ClientConfig, HttpClient, NetError};
use mohona_testserver::{bind, content_byte, serve, Config};
use tokio::sync::mpsc;

async fn start() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve(
            listener,
            Config {
                default_size: 1_000_000,
            },
        )
        .await;
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

fn request(addr: SocketAddr, path: &str, dir: &Path) -> DownloadRequest {
    let mut request = DownloadRequest::new(format!("http://{addr}{path}"), dir);
    // Most tests want every connection at once; adaptive ones set it back.
    request.retry.adaptive = false;
    request
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn assert_generated_content(path: &Path, expected_len: usize) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len(), expected_len);
    for (offset, byte) in data.iter().enumerate() {
        assert_eq!(*byte, content_byte(offset as u64), "byte {offset} differs");
    }
}

#[tokio::test]
async fn downloads_a_whole_file_and_it_matches() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, mut receiver) = mpsc::channel(64);

    let outcome = download(
        &client(),
        &request(addr, "/file?size=3000000", dir.path()),
        Some(&sender),
    )
    .await
    .unwrap();

    assert_eq!(outcome.bytes, 3_000_000);
    assert_eq!(outcome.path, dir.path().join("mohona-test.bin"));
    assert_generated_content(&outcome.path, 3_000_000);
    assert_eq!(file_names(dir.path()), ["mohona-test.bin"]);

    drop(sender);
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    assert!(matches!(
        events.first(),
        Some(Event::Started {
            total: Some(3_000_000),
            resumable: true,
            ..
        })
    ));
    assert!(matches!(
        events.last(),
        Some(Event::Finished {
            bytes: 3_000_000,
            ..
        })
    ));
}

#[tokio::test]
async fn a_second_download_gets_a_numbered_name() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let client = client();
    download(&client, &request(addr, "/file?size=1000", dir.path()), None)
        .await
        .unwrap();
    let second = download(&client, &request(addr, "/file?size=1000", dir.path()), None)
        .await
        .unwrap();
    assert_eq!(second.path, dir.path().join("mohona-test (1).bin"));
    assert_eq!(
        file_names(dir.path()),
        ["mohona-test (1).bin", "mohona-test.bin"]
    );
}

#[tokio::test]
async fn a_cut_connection_is_an_error_and_leaves_no_files() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let result = download(
        &client(),
        &request(addr, "/file?size=100000&drop_after=5000", dir.path()),
        None,
    )
    .await;
    assert!(result.is_err());
    assert!(file_names(dir.path()).is_empty());
}

#[tokio::test]
async fn a_server_that_ignores_ranges_still_downloads() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let outcome = download(
        &client(),
        &request(addr, "/file?size=50000&norange=1", dir.path()),
        None,
    )
    .await
    .unwrap();
    assert_generated_content(&outcome.path, 50_000);
}

#[tokio::test]
async fn a_server_that_rejects_head_still_downloads() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let outcome = download(
        &client(),
        &request(addr, "/file?size=50000&nohead=1", dir.path()),
        None,
    )
    .await
    .unwrap();
    assert_generated_content(&outcome.path, 50_000);
}

#[tokio::test]
async fn redirects_are_followed() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let outcome = download(
        &client(),
        &request(addr, "/redirect?size=10000", dir.path()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(outcome.final_url, format!("http://{addr}/file?size=10000"));
    assert_generated_content(&outcome.path, 10_000);
}

#[tokio::test]
async fn an_empty_file_downloads() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let outcome = download(&client(), &request(addr, "/file?size=0", dir.path()), None)
        .await
        .unwrap();
    assert_eq!(outcome.bytes, 0);
    assert_generated_content(&outcome.path, 0);
}

#[tokio::test]
async fn a_missing_page_is_a_404_error_and_creates_nothing() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let error = download(&client(), &request(addr, "/nope", dir.path()), None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Net(NetError::Status { status: 404, .. })
    ));
    assert!(file_names(dir.path()).is_empty());
}

#[tokio::test]
async fn a_big_file_is_split_over_several_connections_and_matches() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    let size = 20 * 1024 * 1024 + 12_345;
    let mut req = request(addr, &format!("/file?size={size}"), dir.path());
    req.connections = 8;

    let outcome = download(&client(), &req, Some(&sender)).await.unwrap();
    assert_eq!(outcome.bytes, size as u64);
    assert!(outcome.connections >= 8, "used {}", outcome.connections);
    assert_generated_content(&outcome.path, size);
    assert_eq!(file_names(dir.path()), ["mohona-test.bin"]);

    drop(sender);
    let mut started_with = None;
    let mut last_progress = 0;
    while let Some(event) = receiver.recv().await {
        match event {
            Event::Started { connections, .. } => started_with = Some(connections),
            Event::Progress { downloaded, .. } => {
                assert!(downloaded >= last_progress, "progress went backwards");
                last_progress = downloaded;
            }
            _ => {}
        }
    }
    assert_eq!(started_with, Some(8));
}

#[tokio::test]
async fn one_connection_is_a_single_stream() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut req = request(addr, "/file?size=5000000", dir.path());
    req.connections = 1;
    let outcome = download(&client(), &req, None).await.unwrap();
    assert_eq!(outcome.connections, 1);
    assert_generated_content(&outcome.path, 5_000_000);
}

#[tokio::test]
async fn a_slow_segment_is_split_and_the_file_is_still_exact() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    // The first segment crawls at 200 KB/s, so the other connections finish
    // early and must take over half of what is left of it.
    let size = 8 * 1024 * 1024;
    let mut req = request(
        addr,
        &format!("/file?size={size}&slow_start=200000"),
        dir.path(),
    );
    req.connections = 4;
    let outcome = download(&client(), &req, None).await.unwrap();
    assert!(outcome.connections > 4, "no split: {}", outcome.connections);
    assert_generated_content(&outcome.path, size);
}

/// Short waits, so tests about failures do not take long.
fn quick_retries() -> RetryPolicy {
    RetryPolicy {
        base_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(50),
        max_attempts: 3,
        max_throttles: 3,
        adaptive: false,
        sample_interval: Duration::from_millis(20),
    }
}

#[tokio::test]
async fn connections_that_keep_getting_cut_still_finish_the_file() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    // Every response stops after 100 kB, so each segment needs many requests,
    // each continuing at the byte where the last one stopped.
    let size = 1_500_000;
    let mut req = request(
        addr,
        &format!("/file?size={size}&drop_after=100000"),
        dir.path(),
    );
    req.connections = 3;
    req.retry = RetryPolicy {
        max_attempts: 2,
        ..quick_retries()
    };
    let outcome = download(&client(), &req, None).await.unwrap();
    assert_generated_content(&outcome.path, size);
    assert_eq!(file_names(dir.path()), ["mohona-test.bin"]);
}

#[tokio::test]
async fn gives_up_when_no_bytes_ever_arrive() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    // The connection is closed right after the headers, every time.
    let mut req = request(addr, "/file?size=3000000&drop_after=0", dir.path());
    req.connections = 3;
    req.retry = quick_retries();
    assert!(download(&client(), &req, None).await.is_err());
    assert!(file_names(dir.path()).is_empty());
}

#[tokio::test]
async fn a_server_that_limits_connections_gets_fewer_of_them() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    // The server serves two connections at a time and answers 429 to the
    // rest. Mohona asks for 8, backs off, and finishes with two.
    let size = 16 * 1024 * 1024 + 7;
    let mut req = request(
        addr,
        &format!("/file?size={size}&max_conns=2&limit=8000000"),
        dir.path(),
    );
    req.connections = 8;
    req.retry = quick_retries();

    let outcome = download(&client(), &req, Some(&sender)).await.unwrap();
    assert_generated_content(&outcome.path, size);
    assert_eq!(file_names(dir.path()), ["mohona-test.bin"]);

    drop(sender);
    let mut lowest = usize::MAX;
    while let Some(event) = receiver.recv().await {
        if let Event::Limited { connections } = event {
            lowest = lowest.min(connections);
        }
    }
    assert!(
        lowest <= 2,
        "connections were never lowered to the limit ({lowest})"
    );
}

#[tokio::test]
async fn gives_up_when_the_server_never_lets_a_connection_in() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut req = request(addr, "/file?size=3000000&max_conns=0", dir.path());
    req.connections = 4;
    req.retry = quick_retries();
    let error = download(&client(), &req, None).await.unwrap_err();
    assert!(
        matches!(&error, Error::Net(NetError::Status { status: 429, .. })),
        "{error}"
    );
    assert!(file_names(dir.path()).is_empty());
}

fn adaptive_policy() -> RetryPolicy {
    RetryPolicy {
        adaptive: true,
        sample_interval: Duration::from_millis(100),
        ..RetryPolicy::default()
    }
}

#[tokio::test]
async fn adds_connections_when_each_one_is_throttled() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    // Each connection gets 2 MB/s, so more connections mean more speed:
    // one connection would need 15 s for 30 MB.
    let size = 30_000_000;
    let mut req = request(
        addr,
        &format!("/file?size={size}&limit=2000000"),
        dir.path(),
    );
    req.connections = 16;
    req.retry = adaptive_policy();
    let started = std::time::Instant::now();
    let outcome = download(&client(), &req, None).await.unwrap();
    let took = started.elapsed();
    assert_generated_content(&outcome.path, size);
    assert!(took < Duration::from_secs(8), "took {took:?}");
}

#[tokio::test]
async fn backs_off_when_more_connections_make_it_slower() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    // One connection runs at 4 MB/s; with n open, all of them together get
    // only 4/n MB/s. Using 16 connections would take about 100 s; the
    // tuner must settle on one or two and finish in a few seconds.
    let size = 16_000_000;
    let mut req = request(
        addr,
        &format!("/file?size={size}&crowded=4000000"),
        dir.path(),
    );
    req.connections = 16;
    req.retry = adaptive_policy();
    let started = std::time::Instant::now();
    let outcome = download(&client(), &req, None).await.unwrap();
    let took = started.elapsed();
    assert_generated_content(&outcome.path, size);
    assert!(took < Duration::from_secs(12), "took {took:?}");
}

#[tokio::test]
async fn gives_up_on_a_connection_the_server_ignores_and_finishes_with_one() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, mut receiver) = mpsc::channel(256);
    // Each connection runs at 4 MB/s, but the server ignores any connection
    // beyond the first. The tuner's second connection hangs; it must be
    // given up on (not waited for) and the first one must carry on alone.
    let size = 16_000_000;
    let mut req = request(
        addr,
        &format!("/file?size={size}&limit=4000000&hang_above=1"),
        dir.path(),
    );
    req.connections = 8;
    req.retry = adaptive_policy();
    let started = std::time::Instant::now();
    let outcome = download(&client(), &req, Some(&sender)).await.unwrap();
    let took = started.elapsed();
    assert_generated_content(&outcome.path, size);
    assert!(took < Duration::from_secs(12), "took {took:?}");

    drop(sender);
    let mut notes = Vec::new();
    while let Some(event) = receiver.recv().await {
        if let Event::Note(text) = event {
            notes.push(text);
        }
    }
    assert!(
        notes
            .iter()
            .any(|note| note.contains("did not make it faster")),
        "notes: {notes:?}"
    );
}

/// Start a download, cancel it after `after`, and return the error it gave.
async fn download_then_cancel(
    request: &mut DownloadRequest,
    after: Duration,
) -> Result<mohona_core::Outcome, Error> {
    let cancel = CancelToken::new();
    request.cancel = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        cancel.cancel();
    });
    download(&client(), request, None).await
}

/// Run a download and report how many bytes it said were already on disk.
async fn download_reporting_resume(request: &DownloadRequest) -> (mohona_core::Outcome, u64) {
    let (sender, mut receiver) = mpsc::channel(256);
    let outcome = download(&client(), request, Some(&sender)).await.unwrap();
    drop(sender);
    let mut resumed = None;
    while let Some(event) = receiver.recv().await {
        if let Event::Started { resumed: bytes, .. } = event {
            resumed = Some(bytes);
        }
    }
    (outcome, resumed.expect("a Started event"))
}

#[tokio::test]
async fn a_cancelled_download_keeps_its_progress_and_resumes() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 12_000_000;
    let path = format!("/file?size={size}&limit=3000000");

    let mut first = request(addr, &path, dir.path());
    first.connections = 1;
    let error = download_then_cancel(&mut first, Duration::from_millis(1500))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Cancelled), "{error}");
    assert_eq!(
        file_names(dir.path()),
        ["mohona-test.bin.part", "mohona-test.bin.part.mohona"]
    );
    let (saved, total) = partial_progress(dir.path(), &first.url).unwrap();
    assert_eq!(total, size as u64);
    assert!(saved > 0 && saved < total, "saved {saved} of {total}");

    let mut second = request(addr, &path, dir.path());
    second.connections = 1;
    let (outcome, resumed) = download_reporting_resume(&second).await;
    assert_eq!(
        resumed, saved,
        "it should continue exactly where it stopped"
    );
    assert_generated_content(&outcome.path, size);
    assert_eq!(file_names(dir.path()), ["mohona-test.bin"]);
    assert!(partial_progress(dir.path(), &second.url).is_none());
}

#[tokio::test]
async fn resuming_works_when_the_file_was_split_over_several_connections() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 20 * 1024 * 1024 + 321;
    let path = format!("/file?size={size}&limit=2000000");

    let mut first = request(addr, &path, dir.path());
    first.connections = 4;
    let error = download_then_cancel(&mut first, Duration::from_millis(1800))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Cancelled));
    let (saved, _) = partial_progress(dir.path(), &first.url).unwrap();
    assert!(saved > 0);

    let mut second = request(addr, &path, dir.path());
    second.connections = 4;
    let (outcome, resumed) = download_reporting_resume(&second).await;
    assert_eq!(resumed, saved);
    assert_generated_content(&outcome.path, size);
    assert_eq!(file_names(dir.path()), ["mohona-test.bin"]);
}

#[tokio::test]
async fn a_download_that_changed_on_the_server_starts_over() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let size = 8_000_000;
    let path = format!("/file?size={size}&limit=3000000");

    let mut first = request(addr, &path, dir.path());
    first.connections = 1;
    let _ = download_then_cancel(&mut first, Duration::from_millis(1200)).await;
    let ledger_path = dir.path().join("mohona-test.bin.part.mohona");
    // Pretend the server's file had another ETag when the ledger was written.
    let text = std::fs::read_to_string(&ledger_path).unwrap();
    assert!(text.contains(&format!("mohona-{size}-1")));
    std::fs::write(
        &ledger_path,
        text.replace(&format!("mohona-{size}-1"), &format!("mohona-{size}-9")),
    )
    .unwrap();

    let mut second = request(addr, &path, dir.path());
    second.connections = 1;
    let (outcome, resumed) = download_reporting_resume(&second).await;
    assert_eq!(resumed, 0, "stale data must not be reused");
    assert_generated_content(&outcome.path, size);
    assert_eq!(file_names(dir.path()), ["mohona-test.bin"]);
}

#[tokio::test]
async fn unfinished_downloads_of_different_urls_do_not_mix() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();

    let mut first = request(addr, "/file?size=12000000&limit=3000000", dir.path());
    first.connections = 1;
    let _ = download_then_cancel(&mut first, Duration::from_millis(1000)).await;
    let (saved_before, _) = partial_progress(dir.path(), &first.url).unwrap();

    // Another file with the same name must not touch the first one's data.
    let other = request(addr, "/file?size=3000000", dir.path());
    let outcome = download(&client(), &other, None).await.unwrap();
    assert_eq!(outcome.path, dir.path().join("mohona-test (1).bin"));
    assert_generated_content(&outcome.path, 3_000_000);
    assert_eq!(
        partial_progress(dir.path(), &first.url).unwrap().0,
        saved_before
    );
}

#[tokio::test]
async fn an_edited_ledger_cannot_send_the_file_outside_the_folder() {
    let addr = start().await;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("downloads");
    let path = "/file?size=6000000&limit=3000000";

    let mut first = request(addr, path, &dir);
    first.connections = 1;
    let _ = download_then_cancel(&mut first, Duration::from_millis(1000)).await;
    let ledger_path = dir.join("mohona-test.bin.part.mohona");
    let text = std::fs::read_to_string(&ledger_path).unwrap();
    std::fs::write(
        &ledger_path,
        text.replace("\"mohona-test.bin\"", "\"../evil.bin\""),
    )
    .unwrap();

    let mut second = request(addr, path, &dir);
    second.connections = 1;
    let outcome = download(&client(), &second, None).await.unwrap();
    assert_eq!(outcome.path, dir.join("evil.bin"));
    assert!(!root.path().join("evil.bin").exists());
}

#[tokio::test]
async fn cancelling_before_the_start_leaves_nothing() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let req = request(addr, "/file?size=3000000", dir.path());
    req.cancel.cancel();
    assert!(matches!(
        download(&client(), &req, None).await,
        Err(Error::Cancelled)
    ));
    assert!(file_names(dir.path()).is_empty());
}
