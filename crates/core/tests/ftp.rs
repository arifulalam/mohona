//! Downloads from an FTP server.

use std::path::Path;

use mohona_core::{download, CancelToken, DownloadRequest, Error};
use mohona_net::{ClientConfig, HttpClient};
use mohona_net::{Credentials, NetError};
use mohona_testserver::content_byte;
use mohona_testserver::ftp::{start_ftp, FtpConfig, FtpHandle};

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap()
}

fn request(server: &FtpHandle, path: &str, dir: &Path) -> DownloadRequest {
    let mut request = DownloadRequest::new(format!("ftp://{}{path}", server.addr), dir);
    request.retry.adaptive = false;
    request
}

fn assert_content(path: &Path, size: usize) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len(), size);
    for (i, byte) in data.iter().enumerate() {
        assert_eq!(*byte, content_byte(i as u64), "byte {i}");
    }
}

#[tokio::test]
async fn a_file_is_downloaded_in_pieces_over_several_connections() {
    let server = start_ftp(FtpConfig {
        size: 3_000_000,
        ..FtpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(&server, "/pub/data.bin", dir.path());
    r.connections = 4;
    let outcome = download(&client(), &r, None).await.unwrap();
    assert_eq!(outcome.path.file_name().unwrap(), "data.bin");
    assert_content(&outcome.path, 3_000_000);
    // More than one piece was asked for, each starting where it should.
    let retrievals = server.retrievals.lock().unwrap().clone();
    assert!(retrievals.len() >= 2, "{retrievals:?}");
    assert!(retrievals.iter().any(|(_, offset)| *offset > 0));
    // Anonymous login.
    assert!(server
        .logins
        .lock()
        .unwrap()
        .iter()
        .all(|u| u == "anonymous"));
}

#[tokio::test]
async fn a_server_that_cannot_resume_gets_one_plain_download() {
    let server = start_ftp(FtpConfig {
        size: 1_500_000,
        no_rest: true,
        ..FtpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(&server, "/a.bin", dir.path());
    r.connections = 4;
    let outcome = download(&client(), &r, None).await.unwrap();
    assert_content(&outcome.path, 1_500_000);
    assert_eq!(server.retrievals.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn old_servers_without_epsv_work_and_their_private_address_is_ignored() {
    let server = start_ftp(FtpConfig {
        size: 600_000,
        no_epsv: true,
        ..FtpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let outcome = download(&client(), &request(&server, "/a.bin", dir.path()), None)
        .await
        .unwrap();
    assert_content(&outcome.path, 600_000);
}

#[tokio::test]
async fn logins_in_the_address_and_in_the_context() {
    let server = start_ftp(FtpConfig {
        size: 100_000,
        login: Some(("pat".into(), "p@ss".into())),
        ..FtpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();

    // Without a login the server says no, with a hint about what to do.
    let err = download(&client(), &request(&server, "/a.bin", dir.path()), None)
        .await
        .unwrap_err();
    let Error::Net(NetError::Ftp { code: 530, .. }) = &err else {
        panic!("{err:?}");
    };
    assert!(err.to_string().contains("-u user:password"));

    // Wrong password.
    let mut r = request(&server, "/a.bin", dir.path());
    r.context.credentials = Some(Credentials {
        username: "pat".into(),
        password: "nope".into(),
    });
    assert!(download(&client(), &r, None).await.is_err());

    // Right login in the context.
    let mut r = request(&server, "/a.bin", dir.path());
    r.context.credentials = Some(Credentials {
        username: "pat".into(),
        password: "p@ss".into(),
    });
    let outcome = download(&client(), &r, None).await.unwrap();
    assert_content(&outcome.path, 100_000);

    // Right login in the address (with the @ written as %40).
    let dir2 = tempfile::tempdir().unwrap();
    let r = DownloadRequest::new(
        format!("ftp://pat:p%40ss@{}/b.bin", server.addr),
        dir2.path(),
    );
    let outcome = download(&client(), &r, None).await.unwrap();
    assert_content(&outcome.path, 100_000);
}

#[tokio::test]
async fn a_missing_file_is_a_clear_error_and_not_retried() {
    let server = start_ftp(FtpConfig::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let before = server.connections.load(std::sync::atomic::Ordering::SeqCst);
    let err = download(&client(), &request(&server, "/nope.txt", dir.path()), None)
        .await
        .unwrap_err();
    let Error::Net(NetError::Ftp { code: 550, .. }) = &err else {
        panic!("{err:?}");
    };
    assert!(err.to_string().contains("not there"));
    let used = server.connections.load(std::sync::atomic::Ordering::SeqCst) - before;
    assert_eq!(used, 1, "a 550 is final: no retries");
}

#[tokio::test]
async fn a_busy_server_is_given_fewer_connections() {
    let server = start_ftp(FtpConfig {
        size: 4_000_000,
        max_connections: Some(2),
        ..FtpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(&server, "/big.bin", dir.path());
    r.connections = 6;
    r.retry.adaptive = true;
    r.retry.base_delay = std::time::Duration::from_millis(50);
    let outcome = download(&client(), &r, None).await.unwrap();
    assert_content(&outcome.path, 4_000_000);
}

#[tokio::test]
async fn a_stopped_ftp_download_continues_from_where_it_was() {
    let server = start_ftp(FtpConfig {
        size: 6_000_000,
        chunk_delay: Some(std::time::Duration::from_millis(2)),
        ..FtpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancelToken::new();
    let mut r = request(&server, "/big.bin", dir.path());
    r.connections = 2;
    r.cancel = cancel.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let client = client();
    let first = {
        let (client, r) = (client.clone(), r.clone());
        tokio::spawn(async move { download(&client, &r, Some(&tx)).await })
    };
    // Stop it once some data has arrived.
    while let Some(event) = rx.recv().await {
        if let mohona_core::Event::Progress { downloaded, .. } = event {
            if downloaded > 500_000 {
                cancel.cancel();
                break;
            }
        }
    }
    while rx.recv().await.is_some() {}
    let stopped = first.await.unwrap();
    assert!(matches!(stopped, Err(Error::Cancelled)), "{stopped:?}");
    assert!(mohona_core::partial_progress(dir.path(), &r.url).is_some());

    let mut again = request(&server, "/big.bin", dir.path());
    again.connections = 2;
    let outcome = download(&client, &again, None).await.unwrap();
    assert_content(&outcome.path, 6_000_000);
}

#[tokio::test]
async fn a_missing_server_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = DownloadRequest::new("ftp://127.0.0.1:9/a.bin", dir.path());
    r.retry.max_attempts = 1;
    assert!(download(&client(), &r, None).await.is_err());
}

#[tokio::test]
async fn checksums_work_for_ftp_files_too() {
    let server = start_ftp(FtpConfig {
        size: 50_000,
        ..FtpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(&server, "/a.bin", dir.path());
    r.context.checksum = Some(mohona_net::Checksum::parse(&"ab".repeat(32)).unwrap());
    let err = download(&client(), &r, None).await.unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err:?}");
}
