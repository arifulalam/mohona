//! Downloads over SFTP.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mohona_core::{download, CancelToken, DownloadRequest, Error};
use mohona_net::{
    ClientConfig, Credentials, HttpClient, NetError, RequestContext, SshOptions, TlsOptions,
};
use mohona_testserver::content_byte;
use mohona_testserver::sftp::{generate_key_pair, start_sftp, SftpConfig, SftpHandle};

const SIZE: usize = 3_000_000;

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap()
}

async fn server() -> SftpHandle {
    start_sftp(SftpConfig {
        size: SIZE as u64,
        ..SftpConfig::default()
    })
    .await
}

fn password() -> Option<Credentials> {
    Some(Credentials {
        username: "pat".into(),
        password: "secret".into(),
    })
}

fn request(server: &SftpHandle, dir: &Path, ssh: SshOptions) -> DownloadRequest {
    let mut request = DownloadRequest::new(
        format!("sftp://127.0.0.1:{}/home/pat/data.bin", server.addr.port()),
        dir,
    );
    request.retry.adaptive = false;
    request.retry.max_attempts = 1;
    request.connections = 4;
    request.context = RequestContext {
        credentials: password(),
        ssh: Some(ssh),
        ..RequestContext::default()
    };
    request
}

fn pinned(server: &SftpHandle) -> SshOptions {
    SshOptions {
        host_key: Some(server.fingerprint.clone()),
        ..SshOptions::default()
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
async fn a_file_comes_in_pieces_with_a_password_and_a_pinned_host_key() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(&server, dir.path(), pinned(&server));
    let outcome = download(&client(), &r, None).await.unwrap();
    assert_eq!(outcome.path.file_name().unwrap(), "data.bin");
    assert_content(&outcome.path);
    let reads = server.reads.lock().unwrap().clone();
    assert!(
        reads.iter().any(|(_, offset)| *offset > 1_000_000),
        "pieces start further in"
    );
    assert!(server.connections.load(Ordering::SeqCst) >= 2);
    assert_eq!(
        server.logins.lock().unwrap()[0],
        ("pat".to_string(), "password")
    );
}

#[tokio::test]
async fn an_unknown_server_is_refused_before_any_password_is_sent() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("known_hosts");
    std::fs::write(&known, "").unwrap();
    let r = request(
        &server,
        dir.path(),
        SshOptions {
            known_hosts: Some(known.to_str().unwrap().into()),
            ..SshOptions::default()
        },
    );
    let err = download(&client(), &r, None).await.unwrap_err();
    let Error::Net(NetError::HostKey(_)) = &err else {
        panic!("{err:?}");
    };
    let text = err.to_string();
    assert!(text.contains(&server.fingerprint), "{text}");
    assert!(
        text.contains("--host-key") && text.contains("--trust-new-host"),
        "{text}"
    );
    assert!(
        server.logins.lock().unwrap().is_empty(),
        "no password was offered"
    );
    assert_eq!(
        server.connections.load(Ordering::SeqCst),
        1,
        "final, not retried"
    );
}

#[tokio::test]
async fn a_new_server_can_be_remembered_and_is_then_known() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("ssh/known_hosts");
    let options = |trust: bool| SshOptions {
        known_hosts: Some(known.to_str().unwrap().into()),
        trust_new_host: trust,
        ..SshOptions::default()
    };
    let out1 = tempfile::tempdir().unwrap();
    let r = request(&server, out1.path(), options(true));
    assert_content(&download(&client(), &r, None).await.unwrap().path);
    let recorded = std::fs::read_to_string(&known).unwrap();
    assert!(
        recorded.contains(&format!("[127.0.0.1]:{}", server.addr.port())),
        "{recorded}"
    );

    // Next time no trust option is needed.
    let out2 = tempfile::tempdir().unwrap();
    let r = request(&server, out2.path(), options(false));
    assert_content(&download(&client(), &r, None).await.unwrap().path);
}

#[tokio::test]
async fn a_changed_key_is_a_warning_and_never_accepted_even_with_trust_new_host() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("known_hosts");
    let (_, other_public) = generate_key_pair(None);
    std::fs::write(
        &known,
        format!("[127.0.0.1]:{} {}\n", server.addr.port(), other_public),
    )
    .unwrap();
    let r = request(
        &server,
        dir.path(),
        SshOptions {
            known_hosts: Some(known.to_str().unwrap().into()),
            trust_new_host: true,
            ..SshOptions::default()
        },
    );
    let err = download(&client(), &r, None).await.unwrap_err();
    assert!(err.to_string().contains("not the one recorded"), "{err}");
    assert!(server.logins.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_pinned_key_must_match() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let r = request(
        &server,
        dir.path(),
        SshOptions {
            host_key: Some("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into()),
            ..SshOptions::default()
        },
    );
    let err = download(&client(), &r, None).await.unwrap_err();
    assert!(
        err.to_string().contains("you gave with --host-key"),
        "{err}"
    );
    assert!(server.logins.lock().unwrap().is_empty());
}

#[tokio::test]
async fn insecure_accepts_any_host_key() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(&server, dir.path(), SshOptions::default());
    r.context.tls = Some(TlsOptions {
        insecure: true,
        ..TlsOptions::default()
    });
    assert_content(&download(&client(), &r, None).await.unwrap().path);
}

#[tokio::test]
async fn a_wrong_password_is_final() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(&server, dir.path(), pinned(&server));
    r.context.credentials = Some(Credentials {
        username: "pat".into(),
        password: "nope".into(),
    });
    let err = download(&client(), &r, None).await.unwrap_err();
    let Error::Net(NetError::SshLogin(_)) = &err else {
        panic!("{err:?}");
    };
    assert!(err.to_string().contains("refused the login"), "{err}");
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn key_login_works_and_a_protected_key_without_its_passphrase_says_so() {
    let (private, public) = generate_key_pair(None);
    let server = start_sftp(SftpConfig {
        size: SIZE as u64,
        password: None,
        authorized_key: Some(public),
        ..SftpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("id_ed25519");
    std::fs::write(&key, private).unwrap();
    let mut r = request(
        &server,
        dir.path(),
        SshOptions {
            key_file: Some(key.to_str().unwrap().into()),
            ..pinned(&server)
        },
    );
    r.context.credentials = None;
    r.dest_dir = dir.path().join("out");
    r.url = format!(
        "sftp://pat@127.0.0.1:{}/home/pat/data.bin",
        server.addr.port()
    );
    assert_content(&download(&client(), &r, None).await.unwrap().path);
    assert_eq!(server.logins.lock().unwrap()[0].1, "key");

    // A key with a passphrase, none given.
    let (locked, _) = generate_key_pair(Some("hunter2"));
    let locked_path = dir.path().join("locked");
    std::fs::write(&locked_path, locked).unwrap();
    let mut r2 = r.clone();
    r2.dest_dir = dir.path().join("out2");
    r2.context.ssh.as_mut().unwrap().key_file = Some(locked_path.to_str().unwrap().into());
    let err = download(&client(), &r2, None).await.unwrap_err();
    assert!(err.to_string().contains("MOHONA_KEY_PASSPHRASE"), "{err}");
}

#[tokio::test]
async fn a_missing_file_is_a_clear_final_error() {
    let server = server().await;
    let dir = tempfile::tempdir().unwrap();
    let mut r = request(&server, dir.path(), pinned(&server));
    r.url = format!(
        "sftp://127.0.0.1:{}/home/pat/missing.txt",
        server.addr.port()
    );
    let err = download(&client(), &r, None).await.unwrap_err();
    assert!(err.to_string().contains("the file is not there"), "{err}");
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_stopped_sftp_download_continues() {
    let server = start_sftp(SftpConfig {
        size: 20_000_000,
        read_delay: Some(Duration::from_millis(3)),
        ..SftpConfig::default()
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancelToken::new();
    let mut r = request(&server, dir.path(), pinned(&server));
    r.connections = 2;
    r.cancel = cancel.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let c = client();
    let first = {
        let (c, r) = (c.clone(), r.clone());
        tokio::spawn(async move { download(&c, &r, Some(&tx)).await })
    };
    while let Some(event) = rx.recv().await {
        if let mohona_core::Event::Progress { downloaded, .. } = event {
            if downloaded > 4_000_000 {
                cancel.cancel();
                break;
            }
        }
    }
    while rx.recv().await.is_some() {}
    assert!(matches!(first.await.unwrap(), Err(Error::Cancelled)));
    assert!(
        mohona_core::partial_progress(dir.path(), &r.url).is_some(),
        "no saved progress; the folder holds: {:?}",
        std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (e.file_name(), e.metadata().unwrap().len())
            })
            .collect::<Vec<_>>()
    );

    let mut again = request(&server, dir.path(), pinned(&server));
    again.connections = 2;
    let outcome = download(&c, &again, None).await.unwrap();
    let data = std::fs::read(&outcome.path).unwrap();
    assert_eq!(data.len(), 20_000_000);
    assert!(data
        .iter()
        .enumerate()
        .all(|(i, b)| *b == content_byte(i as u64)));
}
