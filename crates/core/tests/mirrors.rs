//! One file, several addresses.

use std::net::SocketAddr;
use std::path::Path;

use mohona_core::{download, DownloadRequest, Event};
use mohona_net::{ClientConfig, HttpClient, ProxyChoice};
use mohona_testserver::proxy::{start_http_proxy, ProxyHandle};
use mohona_testserver::{bind, content_byte, serve, Config};
use tokio::sync::mpsc;

const SIZE: usize = 4_000_000;

async fn start() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve(
            listener,
            Config {
                default_size: SIZE as u64,
            },
        )
        .await;
    });
    addr
}

/// A client whose requests all pass through a counting proxy, so each test
/// can see which address was asked for how many things.
async fn watched() -> (HttpClient, ProxyHandle) {
    let proxy = start_http_proxy(None).await;
    let client = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    let context = mohona_net::RequestContext {
        proxy: Some(ProxyChoice::parse(&format!("http://{}", proxy.addr)).unwrap()),
        ..Default::default()
    };
    (client.with_context(&context).unwrap(), proxy)
}

fn requests_to(proxy: &ProxyHandle, server: SocketAddr) -> usize {
    let wanted = server.to_string();
    proxy
        .targets
        .lock()
        .unwrap()
        .iter()
        .filter(|t| **t == wanted)
        .count()
}

fn request(url: String, mirrors: Vec<String>, dir: &Path) -> DownloadRequest {
    let mut request = DownloadRequest::new(url, dir);
    request.retry.adaptive = false;
    request.connections = 4;
    request.retry.base_delay = std::time::Duration::from_millis(10);
    request.context.mirrors = mirrors;
    request
}

fn assert_generated(path: &Path) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len(), SIZE);
    for (offset, byte) in data.iter().enumerate() {
        assert_eq!(*byte, content_byte(offset as u64), "byte {offset}");
    }
}

async fn run(
    client: &HttpClient,
    request: &DownloadRequest,
) -> (
    Result<mohona_core::Outcome, mohona_core::Error>,
    Vec<String>,
) {
    let (tx, mut rx) = mpsc::channel(256);
    let result = download(client, request, Some(&tx)).await;
    drop(tx);
    let mut notes = Vec::new();
    while let Some(event) = rx.recv().await {
        if let Event::Note(text) = event {
            notes.push(text);
        }
    }
    (result, notes)
}

#[tokio::test]
async fn segments_are_spread_over_the_addresses() {
    let (a, b) = (start().await, start().await);
    let (client, proxy) = watched().await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(
        format!("http://{a}/file"),
        vec![format!("http://{b}/file")],
        dir.path(),
    );
    let (result, _) = run(&client, &r).await;
    let outcome = result.unwrap();
    assert_generated(&outcome.path);
    assert!(requests_to(&proxy, a) >= 3, "a: {}", requests_to(&proxy, a));
    assert!(requests_to(&proxy, b) >= 3, "b: {}", requests_to(&proxy, b));
}

#[tokio::test]
async fn a_dead_mirror_is_skipped_and_the_download_goes_on() {
    let a = start().await;
    let (client, proxy) = watched().await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(
        format!("http://{a}/file"),
        vec!["http://127.0.0.1:9/file".into()],
        dir.path(),
    );
    let (result, notes) = run(&client, &r).await;
    assert_generated(&result.unwrap().path);
    assert!(
        notes
            .iter()
            .any(|n| n.contains("127.0.0.1:9") && n.contains("not used")),
        "{notes:?}"
    );
    assert_eq!(requests_to(&proxy, "127.0.0.1:9".parse().unwrap()), 0);
}

#[tokio::test]
async fn a_mirror_with_another_size_is_not_used() {
    let (a, b) = (start().await, start().await);
    let (client, proxy) = watched().await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(
        format!("http://{a}/file"),
        vec![format!("http://{b}/file?size=1234567")],
        dir.path(),
    );
    let (result, notes) = run(&client, &r).await;
    assert_generated(&result.unwrap().path);
    assert!(
        notes.iter().any(|n| n.contains("different size")),
        "{notes:?}"
    );
    assert_eq!(requests_to(&proxy, b), 1, "only the check, no data");
}

#[tokio::test]
async fn a_mirror_that_fails_while_downloading_is_replaced() {
    let (a, b) = (start().await, start().await);
    let (client, proxy) = watched().await;
    let dir = tempfile::tempdir().unwrap();
    // `broken` passes the check but fails every request for data.
    let r = request(
        format!("http://{a}/file"),
        vec![format!("http://{b}/file?broken")],
        dir.path(),
    );
    let (result, _) = run(&client, &r).await;
    assert_generated(&result.unwrap().path);
    assert!(requests_to(&proxy, b) >= 2, "the broken mirror was tried");
}

#[tokio::test]
async fn the_main_address_failing_hands_over_to_a_mirror() {
    let b = start().await;
    let (client, proxy) = watched().await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(
        "http://127.0.0.1:9/file".into(),
        vec![format!("http://{b}/file")],
        dir.path(),
    );
    let (result, notes) = run(&client, &r).await;
    assert_generated(&result.unwrap().path);
    assert!(
        notes.iter().any(|n| n.contains("using the mirror")),
        "{notes:?}"
    );
    assert!(requests_to(&proxy, b) >= 3);
}

#[tokio::test]
async fn without_any_working_address_the_first_error_is_reported() {
    let (client, _) = watched().await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(
        "http://127.0.0.1:9/file".into(),
        vec!["http://127.0.0.1:8/file".into()],
        dir.path(),
    );
    let (result, _) = run(&client, &r).await;
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("network error") || error.contains("answered"),
        "{error}"
    );
}

#[tokio::test]
async fn every_address_failing_for_data_ends_in_an_error() {
    let (a, b) = (start().await, start().await);
    let (client, _) = watched().await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(
        format!("http://{a}/file?broken"),
        vec![format!("http://{b}/file?broken")],
        dir.path(),
    );
    let (result, _) = run(&client, &r).await;
    assert!(result.is_err());
}
