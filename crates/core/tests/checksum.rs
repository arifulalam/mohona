//! Checking finished downloads against checksums.

use std::net::SocketAddr;
use std::path::Path;

use mohona_core::{download, ChecksumSource, DownloadRequest, Error};
use mohona_net::{Checksum, ClientConfig, HttpClient};
use mohona_testserver::{bind, serve, Config};
use sha2::{Digest, Sha256};

const SIZE: u64 = 200_000;

async fn start() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve(listener, Config { default_size: SIZE }).await;
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
    request.retry.adaptive = false;
    request
}

/// The real SHA-256 of the generated file, worked out here independently.
fn real_sha256() -> String {
    let mut data = vec![0u8; usize::try_from(SIZE).unwrap()];
    mohona_testserver::fill_content(0, &mut data);
    Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_given_checksum_is_checked() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(addr, "/file", dir.path());
    r.context.checksum = Some(Checksum::parse(&real_sha256()).unwrap());
    let outcome = download(&client(), &r, None).await.unwrap();
    let verified = outcome.verified.expect("checked");
    assert_eq!(verified.source, ChecksumSource::Given);
    assert_eq!(verified.checksum.hex, real_sha256());
}

#[tokio::test]
async fn a_wrong_checksum_fails_but_keeps_the_file() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(addr, "/file", dir.path());
    r.context.checksum = Some(Checksum::parse(&"ab".repeat(32)).unwrap());
    let err = download(&client(), &r, None).await.unwrap_err();
    let Error::ChecksumMismatch {
        expected,
        actual,
        path,
        ..
    } = &err
    else {
        panic!("{err:?}");
    };
    assert_eq!(expected, &"ab".repeat(32));
    assert_eq!(actual, &real_sha256());
    assert!(path.exists(), "the file must be kept");
    assert_eq!(std::fs::metadata(path).unwrap().len(), SIZE);
    assert_eq!(
        files(dir.path()),
        vec!["mohona-test.bin"],
        "nothing else is left over"
    );
    assert!(err.to_string().contains("was kept at"));
}

#[tokio::test]
async fn hashes_the_server_announces_are_checked_automatically() {
    let addr = start().await;
    for (query, algorithm) in [("content_md5=right", "md5"), ("digest=right", "sha256")] {
        let dir = tempfile::tempdir().unwrap();
        let r = request(addr, &format!("/file?{query}"), dir.path());
        let verified = download(&client(), &r, None)
            .await
            .unwrap()
            .verified
            .expect("announced hash is checked");
        assert_eq!(verified.source, ChecksumSource::Header);
        assert_eq!(verified.checksum.algorithm.name(), algorithm);
    }
    // A file the server promised something else about fails.
    for query in ["content_md5=wrong", "digest=wrong"] {
        let dir = tempfile::tempdir().unwrap();
        let r = request(addr, &format!("/file?{query}"), dir.path());
        let err = download(&client(), &r, None).await.unwrap_err();
        assert!(
            matches!(err, Error::ChecksumMismatch { .. }),
            "{query}: {err:?}"
        );
    }
    // Without any announcement nothing is checked.
    let dir = tempfile::tempdir().unwrap();
    let r = request(addr, "/file", dir.path());
    assert!(download(&client(), &r, None)
        .await
        .unwrap()
        .verified
        .is_none());
}

#[tokio::test]
async fn a_given_checksum_wins_over_the_servers() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(addr, "/file?content_md5=wrong", dir.path());
    r.context.checksum = Some(Checksum::parse(&real_sha256()).unwrap());
    let verified = download(&client(), &r, None)
        .await
        .unwrap()
        .verified
        .unwrap();
    assert_eq!(verified.source, ChecksumSource::Given);
}

#[tokio::test]
async fn checksum_files_next_to_the_download_are_used_when_asked() {
    let addr = start().await;
    for style in ["plain", "lone", "list"] {
        let dir = tempfile::tempdir().unwrap();
        let mut r = request(addr, &format!("/sums/{style}/{SIZE}.bin"), dir.path());
        r.context.find_checksum = true;
        let verified = download(&client(), &r, None)
            .await
            .unwrap()
            .verified
            .unwrap_or_else(|| panic!("{style}: not checked"));
        assert!(matches!(verified.source, ChecksumSource::File(ref u) if u.ends_with(".sha256")));
    }
    // A wrong one fails and keeps the file.
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(addr, &format!("/sums/wrong/{SIZE}.bin"), dir.path());
    r.context.find_checksum = true;
    assert!(matches!(
        download(&client(), &r, None).await.unwrap_err(),
        Error::ChecksumMismatch { .. }
    ));
    assert_eq!(files(dir.path()), vec!["mohona-test.bin"]);
}

#[tokio::test]
async fn checksum_files_are_not_looked_for_unless_asked() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    // The checksum file here is wrong, but nobody asked to use it.
    let r = request(addr, &format!("/sums/wrong/{SIZE}.bin"), dir.path());
    assert!(download(&client(), &r, None)
        .await
        .unwrap()
        .verified
        .is_none());
}

#[tokio::test]
async fn a_missing_or_unrelated_checksum_file_is_not_an_error() {
    let addr = start().await;
    for style in ["none", "other"] {
        let dir = tempfile::tempdir().unwrap();
        let mut r = request(addr, &format!("/sums/{style}/{SIZE}.bin"), dir.path());
        r.context.find_checksum = true;
        let outcome = download(&client(), &r, None).await.unwrap();
        assert!(outcome.verified.is_none(), "{style}");
    }
}

#[tokio::test]
async fn a_checked_ranged_download_with_many_connections_matches() {
    let addr = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(addr, &format!("/sums/plain/{SIZE}.bin"), dir.path());
    r.connections = 6;
    r.context.find_checksum = true;
    let outcome = download(&client(), &r, None).await.unwrap();
    assert!(outcome.verified.is_some());
}
