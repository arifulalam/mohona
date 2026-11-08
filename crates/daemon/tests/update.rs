//! Looking for a newer version, against a stand-in for GitHub.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::{serve, Call, Client, DaemonInfo, Response, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

async fn start(dir: &Path) -> (Client, Arc<Notify>) {
    let store = Store::open(&dir.join("test.db")).unwrap();
    let http = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    let manager = Manager::start(store, http, ManagerConfig::default()).unwrap();
    let info_path = dir.join("daemon.json");
    let shutdown = Arc::new(Notify::new());
    tokio::spawn(serve(
        manager,
        ServerConfig {
            info_path: info_path.clone(),
            default_dir: dir.join("dl"),
            stream_program: None,
            shutdown: shutdown.clone(),
        },
    ));
    let started = Instant::now();
    loop {
        if let Ok(info) = DaemonInfo::read(&info_path) {
            if info.pid == std::process::id() {
                return (Client::new(&info), shutdown);
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "daemon did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_newer_release_is_reported_and_nothing_else_is_asked() {
    let github = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = github.local_addr().unwrap();
    let (seen_tx, mut seen) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = github.accept().await.unwrap();
            let seen_tx = seen_tx.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 4096];
                let n = socket.read(&mut buffer).await.unwrap_or(0);
                let line = String::from_utf8_lossy(&buffer[..n])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let _ = seen_tx.send(line.clone());
                let (status, body) = if line.contains("/repos/o/r/releases/latest") {
                    (
                        "200 OK",
                        r#"{"tag_name":"v9.0.0","html_url":"https://github.com/o/r/releases/tag/v9.0.0","body":"Big"}"#,
                    )
                } else {
                    ("404 Not Found", r#"{"message":"Not Found"}"#)
                };
                let answer = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    std::env::set_var("MOHONA_UPDATE_API", format!("http://{addr}"));
    let dir = tempfile::tempdir().unwrap();
    let (client, shutdown) = start(dir.path()).await;

    let Response::Update {
        repo,
        latest,
        newer,
        ..
    } = client.call(Call::GetUpdate).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(repo, "arifulalam/mohona");
    assert!(latest.is_none() && !newer);
    // Emptied, the check is off.
    client
        .call(Call::SetUpdateRepo {
            repo: String::new(),
        })
        .await
        .unwrap();
    assert!(matches!(
        client.call(Call::CheckUpdate).await.unwrap(),
        Response::Error { .. }
    ));
    assert!(matches!(
        client
            .call(Call::SetUpdateRepo {
                repo: "not a repo".into()
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert!(
        seen.try_recv().is_err(),
        "nothing may be asked before a repository is set"
    );

    client
        .call(Call::SetUpdateRepo { repo: "o/r".into() })
        .await
        .unwrap();
    let Response::Update {
        repo,
        latest,
        newer,
        url,
        notes,
        ..
    } = client.call(Call::CheckUpdate).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(repo, "o/r");
    assert_eq!(latest.as_deref(), Some("9.0.0"));
    assert!(newer);
    assert_eq!(
        url.as_deref(),
        Some("https://github.com/o/r/releases/tag/v9.0.0")
    );
    assert_eq!(notes.as_deref(), Some("Big"));
    assert!(seen
        .recv()
        .await
        .unwrap()
        .contains("/repos/o/r/releases/latest"));

    // A repository with no release is an error that says so.
    client
        .call(Call::SetUpdateRepo {
            repo: "o/none".into(),
        })
        .await
        .unwrap();
    match client.call(Call::CheckUpdate).await.unwrap() {
        Response::Error { message } => {
            assert!(message.contains("no published release"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    shutdown.notify_one();
}
