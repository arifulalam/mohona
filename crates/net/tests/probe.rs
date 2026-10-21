//! Probe tests against the local test server.

use std::net::SocketAddr;

use mohona_net::{ClientConfig, HttpClient, NetError};
use mohona_testserver::{bind, serve, Config};

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

#[tokio::test]
async fn probe_reports_everything_the_server_says() {
    let addr = start().await;
    let probe = client()
        .probe(&format!("http://{addr}/file?size=100000"))
        .await
        .unwrap();
    assert_eq!(probe.size, Some(100_000));
    assert!(probe.accepts_ranges);
    assert_eq!(probe.etag.as_deref(), Some("\"mohona-100000-1\""));
    assert_eq!(
        probe.last_modified.as_deref(),
        Some("Wed, 01 Jan 2025 00:00:00 GMT")
    );
    assert_eq!(
        probe.content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(probe.filename.as_deref(), Some("mohona-test.bin"));
}

#[tokio::test]
async fn probe_works_when_the_server_rejects_head() {
    let addr = start().await;
    let probe = client()
        .probe(&format!("http://{addr}/file?size=100000&nohead=1"))
        .await
        .unwrap();
    assert_eq!(probe.size, Some(100_000));
    assert!(probe.accepts_ranges);
    assert_eq!(probe.filename.as_deref(), Some("mohona-test.bin"));
}

#[tokio::test]
async fn probe_sees_that_ranges_are_ignored() {
    let addr = start().await;
    let probe = client()
        .probe(&format!("http://{addr}/file?size=1000&norange=1"))
        .await
        .unwrap();
    assert_eq!(probe.size, Some(1000));
    assert!(!probe.accepts_ranges);
}

#[tokio::test]
async fn probe_follows_redirects() {
    let addr = start().await;
    let probe = client()
        .probe(&format!("http://{addr}/redirect?size=10"))
        .await
        .unwrap();
    assert_eq!(probe.final_url, format!("http://{addr}/file?size=10"));
    assert_eq!(probe.size, Some(10));
}

#[tokio::test]
async fn probe_handles_an_empty_file() {
    let addr = start().await;
    let probe = client()
        .probe(&format!("http://{addr}/file?size=0"))
        .await
        .unwrap();
    assert_eq!(probe.size, Some(0));
}

#[tokio::test]
async fn probe_takes_the_name_from_the_url_when_there_is_no_header() {
    let addr = start().await;
    let probe = client()
        .probe(&format!("http://{addr}/info"))
        .await
        .unwrap();
    assert_eq!(probe.filename.as_deref(), Some("info"));
    assert!(!probe.accepts_ranges);
}

#[tokio::test]
async fn probe_reports_a_missing_page_as_a_status_error() {
    let addr = start().await;
    let error = client()
        .probe(&format!("http://{addr}/nope"))
        .await
        .unwrap_err();
    assert!(matches!(error, NetError::Status { status: 404, .. }));
}

#[tokio::test]
async fn probe_rejects_other_schemes_and_bad_urls() {
    let client = client();
    assert!(matches!(
        client.probe("gopher://example.com/file").await.unwrap_err(),
        NetError::UnsupportedScheme(_)
    ));
    assert!(matches!(
        client.probe("not a url").await.unwrap_err(),
        NetError::InvalidUrl(_)
    ));
}
