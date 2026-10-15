//! End-to-end download tests against the local test server.

use std::net::SocketAddr;
use std::path::Path;

use mohona_core::{download, DownloadRequest, Error, Event};
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
    DownloadRequest::new(format!("http://{addr}{path}"), dir)
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
            Event::Finished { .. } => {}
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

#[tokio::test]
async fn a_cut_segment_fails_the_download_and_leaves_no_files() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut req = request(addr, "/file?size=8000000&drop_after=100000", dir.path());
    req.connections = 4;
    assert!(download(&client(), &req, None).await.is_err());
    assert!(file_names(dir.path()).is_empty());
}
