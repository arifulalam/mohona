//! Encrypted FTP: `ftps://` (encrypted from the start) and `ftpes://`
//! (switches on with AUTH TLS).

use std::path::Path;
use std::sync::atomic::Ordering;

use mohona_core::{download, DownloadRequest, Error};
use mohona_net::{ClientConfig, HttpClient, NetError, RequestContext, TlsOptions};
use mohona_testserver::content_byte;
use mohona_testserver::ftp::{start_ftp, FtpConfig, FtpHandle, FtpTls};
use mohona_testserver::tls::Certificates;

const SIZE: usize = 2_500_000;

async fn server(implicit: bool, certificates: &Certificates) -> FtpHandle {
    start_ftp(FtpConfig {
        size: SIZE as u64,
        tls: Some(FtpTls {
            certificates: certificates.clone(),
            implicit,
        }),
        ..FtpConfig::default()
    })
    .await
}

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap()
}

fn request(
    scheme: &str,
    server: &FtpHandle,
    dir: &Path,
    tls: Option<TlsOptions>,
) -> DownloadRequest {
    let mut request = DownloadRequest::new(
        format!("{scheme}://localhost:{}/pub/a.bin", server.addr.port()),
        dir,
    );
    request.retry.adaptive = false;
    request.retry.max_attempts = 1;
    request.connections = 4;
    request.context = RequestContext {
        tls,
        ..RequestContext::default()
    };
    request
}

fn ca(certificates: &Certificates, dir: &Path) -> TlsOptions {
    let path = dir.join("ca.pem");
    std::fs::write(&path, &certificates.ca_pem).unwrap();
    TlsOptions {
        ca_file: Some(path.to_str().unwrap().into()),
        ..TlsOptions::default()
    }
}

fn assert_content(path: &Path) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len(), SIZE);
    for (i, byte) in data.iter().enumerate() {
        assert_eq!(*byte, content_byte(i as u64), "byte {i}");
    }
}

#[tokio::test]
async fn both_kinds_download_encrypted_when_the_authority_is_trusted() {
    for (scheme, implicit) in [("ftps", true), ("ftpes", false)] {
        let certificates = Certificates::new(false);
        let server = server(implicit, &certificates).await;
        let keep = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let r = request(
            scheme,
            &server,
            dir.path(),
            Some(ca(&certificates, keep.path())),
        );
        let outcome = download(&client(), &r, None).await.unwrap();
        assert_content(&outcome.path);
        assert!(
            server.retrievals.lock().unwrap().len() >= 2,
            "{scheme}: in pieces"
        );
        assert!(
            server.encrypted_transfers.load(Ordering::SeqCst) >= 2,
            "{scheme}"
        );
        assert_eq!(
            server.plain_transfers.load(Ordering::SeqCst),
            0,
            "{scheme}: nothing in the clear"
        );
    }
}

#[tokio::test]
async fn an_untrusted_certificate_is_refused_with_the_way_out_and_without_retries() {
    for (scheme, implicit) in [("ftps", true), ("ftpes", false)] {
        let certificates = Certificates::new(false);
        let server = server(implicit, &certificates).await;
        let dir = tempfile::tempdir().unwrap();
        let r = request(scheme, &server, dir.path(), None);
        let err = download(&client(), &r, None).await.unwrap_err();
        assert!(
            matches!(err, Error::Net(NetError::Tls(_))),
            "{scheme}: {err:?}"
        );
        let text = err.to_string();
        assert!(
            text.contains("not signed by an authority") && text.contains("--ca-file"),
            "{text}"
        );
        assert_eq!(
            server.connections.load(Ordering::SeqCst),
            1,
            "{scheme}: final, not retried"
        );
        assert_eq!(
            server.logins.lock().unwrap().len(),
            0,
            "no password was sent"
        );
    }
}

#[tokio::test]
async fn insecure_skips_the_check() {
    let certificates = Certificates::new(false);
    let server = server(true, &certificates).await;
    let dir = tempfile::tempdir().unwrap();
    let tls = TlsOptions {
        insecure: true,
        ..TlsOptions::default()
    };
    let r = request("ftps", &server, dir.path(), Some(tls));
    assert_content(&download(&client(), &r, None).await.unwrap().path);
}

#[tokio::test]
async fn a_login_is_sent_only_after_the_connection_is_encrypted() {
    let certificates = Certificates::new(false);
    let server = start_ftp(FtpConfig {
        size: SIZE as u64,
        login: Some(("pat".into(), "secret".into())),
        tls: Some(FtpTls {
            certificates: certificates.clone(),
            implicit: false,
        }),
        ..FtpConfig::default()
    })
    .await;
    let keep = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(
        "ftpes",
        &server,
        dir.path(),
        Some(ca(&certificates, keep.path())),
    );
    r.context.credentials = Some(mohona_net::Credentials {
        username: "pat".into(),
        password: "secret".into(),
    });
    assert_content(&download(&client(), &r, None).await.unwrap().path);
    assert_eq!(
        server.logins.lock().unwrap().first().map(String::as_str),
        Some("pat")
    );
}

#[tokio::test]
async fn encrypted_addresses_on_a_plain_server_fail_instead_of_hanging() {
    let server = start_ftp(FtpConfig::default()).await;
    let dir = tempfile::tempdir().unwrap();
    let insecure = Some(TlsOptions {
        insecure: true,
        ..TlsOptions::default()
    });
    // A server that does not know AUTH TLS says so, and nothing is sent in the clear.
    let r = request("ftpes", &server, dir.path(), insecure);
    let err = download(&client(), &r, None).await.unwrap_err();
    assert!(matches!(err, Error::Net(NetError::Ftp { .. })), "{err:?}");
    assert_eq!(server.logins.lock().unwrap().len(), 0);
}
