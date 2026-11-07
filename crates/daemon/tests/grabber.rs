//! The site grabber through the daemon: start, watch, stop.

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

/// A two-page site on this computer.
async fn site() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 4096];
                let n = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = match path.as_str() {
                    "/" => Some(
                        r#"<a href="a.pdf">a</a> <a href="/two.html">two</a> <img src="/p.png">"#,
                    ),
                    "/two.html" => Some(r#"<a href="b.zip">b</a>"#),
                    _ => None,
                };
                let answer = match body {
                    Some(b) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
                        b.len()
                    ),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                };
                let _ = socket.write_all(answer.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    addr
}

async fn finished(client: &Client) -> Response {
    let began = Instant::now();
    loop {
        let r = client.call(Call::GrabStatus).await.unwrap();
        if let Response::Grab { state, .. } = &r {
            if state != "running" {
                return r;
            }
        }
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "the grabber did not finish"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn start_call(url: String, depth: u32, types: Vec<String>) -> Call {
    Call::GrabStart {
        url,
        depth,
        same_site: true,
        types,
        max_pages: None,
        robots: true,
    }
}

#[tokio::test]
async fn the_grabber_lists_files_and_reports_how_it_went() {
    let dir = tempfile::tempdir().unwrap();
    let (client, shutdown) = start(dir.path()).await;
    let addr = site().await;

    assert!(matches!(
        client.call(Call::GrabStatus).await.unwrap(),
        Response::Grab { state, .. } if state == "idle"
    ));

    client
        .call(start_call(format!("http://{addr}/"), 1, vec![]))
        .await
        .unwrap();
    let Response::Grab {
        state,
        pages,
        files,
        found,
        ..
    } = finished(&client).await
    else {
        panic!()
    };
    assert_eq!(state, "done");
    assert_eq!(pages, 2);
    assert_eq!(found, 3);
    let mut names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["a.pdf", "b.zip", "p.png"]);
    assert_eq!(
        files.iter().find(|f| f.name == "p.png").unwrap().group,
        "images"
    );

    // A second run with a filter replaces the first one's result.
    client
        .call(start_call(
            format!("http://{addr}/"),
            1,
            vec!["archives".into()],
        ))
        .await
        .unwrap();
    let Response::Grab { files, .. } = finished(&client).await else {
        panic!()
    };
    assert_eq!(files.len(), 1);

    // A bad address is a failure with a reason, not a crash.
    client
        .call(start_call("ftp://x/y".into(), 1, vec![]))
        .await
        .unwrap();
    let Response::Grab { state, error, .. } = finished(&client).await else {
        panic!()
    };
    assert_eq!(state, "failed");
    assert!(error.unwrap().contains("http"));

    // Stopping keeps what there is.
    client
        .call(start_call(format!("http://{addr}/"), 3, vec![]))
        .await
        .unwrap();
    client.call(Call::GrabStop).await.unwrap();
    let Response::Grab { state, .. } = finished(&client).await else {
        panic!()
    };
    assert!(state == "stopped" || state == "done", "{state}");
    shutdown.notify_one();
}
