//! WebDAV addresses: HTTP(S) under another name, with logins.

use std::net::SocketAddr;
use std::path::Path;

use mohona_core::{download, DownloadRequest};
use mohona_net::{ClientConfig, Credentials, HttpClient, RequestContext, TlsOptions};
use mohona_testserver::tls::{serve_tls, Certificates};
use mohona_testserver::{bind, content_byte, serve, Config};

const SIZE: usize = 1_200_000;

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap()
}

fn assert_content(path: &Path) {
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len(), SIZE);
    assert!(data
        .iter()
        .enumerate()
        .all(|(i, b)| *b == content_byte(i as u64)));
}

fn login() -> Option<Credentials> {
    Some(Credentials {
        username: "mohona".into(),
        password: "secret".into(),
    })
}

async fn plain() -> SocketAddr {
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

#[tokio::test]
async fn webdav_and_dav_addresses_are_http_with_a_login() {
    let addr = plain().await;
    for scheme in ["webdav", "dav", "WebDAV"] {
        let dir = tempfile::tempdir().unwrap();
        let mut r = DownloadRequest::new(format!("{scheme}://{addr}/auth"), dir.path());
        r.connections = 3;
        r.context = RequestContext {
            credentials: login(),
            ..RequestContext::default()
        };
        let outcome = download(&client(), &r, None).await.unwrap();
        assert_content(&outcome.path);
        assert!(
            outcome.final_url.starts_with("http://"),
            "{}",
            outcome.final_url
        );
    }
}

#[tokio::test]
async fn without_the_login_the_message_says_what_to_do() {
    let addr = plain().await;
    let dir = tempfile::tempdir().unwrap();
    let r = DownloadRequest::new(format!("webdav://{addr}/auth"), dir.path());
    let err = download(&client(), &r, None).await.unwrap_err();
    assert!(err.to_string().contains("-u user:password"), "{err}");
}

#[tokio::test]
async fn webdavs_and_davs_are_https_with_the_same_certificate_rules() {
    let certificates = Certificates::new(false);
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = certificates.clone();
    tokio::spawn(async move {
        let _ = serve_tls(
            listener,
            Config {
                default_size: SIZE as u64,
            },
            &served,
        )
        .await;
    });
    let keep = tempfile::tempdir().unwrap();
    let ca = keep.path().join("ca.pem");
    std::fs::write(&ca, &certificates.ca_pem).unwrap();
    for scheme in ["webdavs", "davs"] {
        // Untrusted: refused, like HTTPS.
        let dir = tempfile::tempdir().unwrap();
        let r = DownloadRequest::new(format!("{scheme}://localhost:{port}/file"), dir.path());
        let err = download(&client(), &r, None).await.unwrap_err();
        assert!(err.to_string().contains("--ca-file"), "{scheme}: {err}");
        // With the authority: fine.
        let mut r = DownloadRequest::new(format!("{scheme}://localhost:{port}/file"), dir.path());
        r.context.tls = Some(TlsOptions {
            ca_file: Some(ca.to_str().unwrap().into()),
            ..TlsOptions::default()
        });
        let outcome = download(&client(), &r, None).await.unwrap();
        assert_content(&outcome.path);
        assert!(outcome.final_url.starts_with("https://"));
    }
}
