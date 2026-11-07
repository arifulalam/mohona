//! The virus scan after a download, with a stand-in scanner script.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
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

async fn files() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let body = "x".repeat(100);
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{body}"
                );
                let _ = socket.write_all(answer.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    addr
}

async fn add(client: &Client, url: String) -> i64 {
    match client
        .call(Call::Add {
            url,
            dest_dir: None,
            cookies: None,
            referrer: None,
            user_agent: None,
            headers: vec![],
            sort: false,
            duplicates: None,
            queue: None,
            files: None,
            video: false,
        })
        .await
        .unwrap()
    {
        Response::Added { id } => id,
        other => panic!("{other:?}"),
    }
}

async fn log_of(dir: &Path, id: i64, wanted: &str) -> String {
    let store = Store::open(&dir.join("test.db")).unwrap();
    let began = Instant::now();
    loop {
        if let Ok(lines) = store.log_of(id) {
            if let Some(line) = lines.iter().find(|l| l.message.contains(wanted)) {
                return line.message.clone();
            }
        }
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "no log line with {wanted}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn finished_downloads_are_scanned_and_a_problem_shows_on_the_download() {
    let dir = tempfile::tempdir().unwrap();
    let scanner = dir.path().join("scan.sh");
    std::fs::write(
        &scanner,
        "#!/bin/sh\ncase \"$1\" in *evil*) echo 'Threat: Test.Virus'; exit 2;; esac\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&scanner, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (client, shutdown) = start(dir.path()).await;
    let server = files().await;

    // Off by default: nothing is scanned.
    assert!(matches!(
        client.call(Call::GetScan).await.unwrap(),
        Response::Scan { mode, .. } if mode == "off"
    ));
    assert!(matches!(
        client
            .call(Call::SetScan {
                mode: "custom".into(),
                command: "  ".into()
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert!(matches!(
        client
            .call(Call::SetScan {
                mode: "nope".into(),
                command: String::new()
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    let first = add(&client, format!("http://{server}/before.bin")).await;
    tokio::time::sleep(Duration::from_millis(600)).await;

    client
        .call(Call::SetScan {
            mode: "custom".into(),
            command: scanner.display().to_string(),
        })
        .await
        .unwrap();
    let clean = add(&client, format!("http://{server}/fine.bin")).await;
    assert_eq!(
        log_of(dir.path(), clean, "virus scan").await,
        "virus scan: nothing found"
    );
    let evil = add(&client, format!("http://{server}/evil.bin")).await;
    let said = log_of(dir.path(), evil, "virus scan").await;
    assert!(said.contains("Test.Virus"), "{said}");
    // The download stays completed, and says so on the row.
    let began = Instant::now();
    loop {
        let Response::Downloads { downloads } = client.call(Call::List).await.unwrap() else {
            panic!()
        };
        let row = downloads.iter().find(|d| d.id == evil).unwrap();
        if row
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Test.Virus"))
        {
            assert_eq!(row.status, "completed");
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "no warning on the row"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let Response::Downloads { downloads } = client.call(Call::List).await.unwrap() else {
        panic!()
    };
    assert!(downloads
        .iter()
        .find(|d| d.id == clean)
        .unwrap()
        .error
        .is_none());
    assert!(downloads
        .iter()
        .find(|d| d.id == first)
        .unwrap()
        .error
        .is_none());
    // A scanner that is not there is a note, not a failure.
    client
        .call(Call::SetScan {
            mode: "custom".into(),
            command: "/no/such/scanner".into(),
        })
        .await
        .unwrap();
    let lost = add(&client, format!("http://{server}/lost.bin")).await;
    assert!(log_of(dir.path(), lost, "virus scan not run")
        .await
        .contains("could not be started"));
    shutdown.notify_one();
}
