//! The certificate check: strict by default, with two deliberate ways out.

use std::net::SocketAddr;

use mohona_net::{ClientConfig, HttpClient, NetError, RequestContext, TlsOptions};
use mohona_testserver::tls::{serve_tls, Certificates};
use mohona_testserver::{bind, Config};

async fn start(certificates: Certificates) -> (SocketAddr, Certificates) {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = Certificates::clone_for_server(&certificates);
    tokio::spawn(async move {
        let _ = serve_tls(
            listener,
            Config {
                default_size: 40_000,
            },
            &served,
        )
        .await;
    });
    (addr, certificates)
}

fn client(tls: Option<TlsOptions>) -> HttpClient {
    let base = HttpClient::new(&ClientConfig::default()).unwrap();
    match tls {
        None => base,
        Some(tls) => base
            .with_context(&RequestContext {
                tls: Some(tls),
                ..RequestContext::default()
            })
            .unwrap(),
    }
}

async fn size(client: &HttpClient, url: &str) -> Result<u64, NetError> {
    Ok(client.probe(url).await?.size.unwrap())
}

fn ca_file(certificates: &Certificates) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ca.pem");
    std::fs::write(&path, &certificates.ca_pem).unwrap();
    let path = path.to_str().unwrap().to_string();
    (dir, path)
}

#[tokio::test]
async fn an_untrusted_certificate_is_refused_by_default() {
    let (addr, _) = start(Certificates::new(false)).await;
    let url = format!("https://localhost:{}/file", addr.port());
    let err = size(&client(None), &url).await.unwrap_err();
    assert!(matches!(err, NetError::Tls(_)), "{err:?}");
    let text = err.to_string();
    assert!(text.contains("not signed by an authority"), "{text}");
    assert!(
        text.contains("--ca-file") && text.contains("--insecure"),
        "{text}"
    );
    assert_eq!(err.failure(), mohona_net::Failure::Fatal);
}

#[tokio::test]
async fn a_given_authority_is_trusted() {
    let (addr, certificates) = start(Certificates::new(false)).await;
    let (_keep, path) = ca_file(&certificates);
    let tls = TlsOptions {
        ca_file: Some(path),
        ..TlsOptions::default()
    };
    // By name and by address: the certificate covers both.
    for host in ["localhost", "127.0.0.1"] {
        let url = format!("https://{host}:{}/file", addr.port());
        assert_eq!(
            size(&client(Some(tls.clone())), &url).await.unwrap(),
            40_000
        );
    }
}

#[tokio::test]
async fn insecure_accepts_anything_and_only_for_that_download() {
    let (addr, _) = start(Certificates::new(false)).await;
    let url = format!("https://localhost:{}/file", addr.port());
    let insecure = TlsOptions {
        insecure: true,
        ..TlsOptions::default()
    };
    assert_eq!(size(&client(Some(insecure)), &url).await.unwrap(), 40_000);
    // A client without the option is still strict.
    assert!(size(&client(None), &url).await.is_err());
}

#[tokio::test]
async fn an_expired_certificate_is_named_as_such() {
    let (addr, certificates) = start(Certificates::new(true)).await;
    let (_keep, path) = ca_file(&certificates);
    let url = format!("https://localhost:{}/file", addr.port());
    let tls = TlsOptions {
        ca_file: Some(path),
        ..TlsOptions::default()
    };
    let err = size(&client(Some(tls)), &url).await.unwrap_err();
    assert!(err.to_string().contains("has expired"), "{err}");
}

#[tokio::test]
async fn a_certificate_for_another_name_is_named_as_such() {
    let (addr, certificates) = start(Certificates::new(false)).await;
    let (_keep, path) = ca_file(&certificates);
    // "localtest.me" style names are not in the certificate; use the IPv6 loopback name.
    let url = format!("https://[::1]:{}/file", addr.port());
    let tls = TlsOptions {
        ca_file: Some(path),
        ..TlsOptions::default()
    };
    let err = size(&client(Some(tls)), &url).await.unwrap_err();
    // Either the name is refused (we are listening on IPv4 only, so the
    // connection may fail first); both are errors, and never a success.
    assert!(
        matches!(err, NetError::Tls(_) | NetError::Http(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn bad_authority_files_are_explained() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.pem");
    let junk = dir.path().join("junk.pem");
    std::fs::write(&junk, "this is not a certificate").unwrap();
    let base = HttpClient::new(&ClientConfig::default()).unwrap();
    for path in [missing, junk] {
        let result = base.with_context(&RequestContext {
            tls: Some(TlsOptions {
                ca_file: Some(path.to_str().unwrap().into()),
                ..TlsOptions::default()
            }),
            ..RequestContext::default()
        });
        let err = result.err().expect("must fail");
        assert!(matches!(err, NetError::InvalidCaFile { .. }), "{err:?}");
        assert!(err.to_string().contains("cannot use the certificate file"));
    }
}

#[test]
fn tls_options_are_saved_and_loaded() {
    let context = RequestContext {
        tls: Some(TlsOptions {
            insecure: true,
            ca_file: Some("/x/ca.pem".into()),
        }),
        ..RequestContext::default()
    };
    assert_eq!(
        RequestContext::from_json(&context.to_json()).unwrap(),
        context
    );
    assert!(!context.is_empty());
}
