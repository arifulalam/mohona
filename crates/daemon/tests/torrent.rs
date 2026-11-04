//! A magnet link added through the daemon, downloaded from a seeder on this computer
//! that a tiny tracker points to.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use librqbit::{
    create_torrent, AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session,
    SessionOptions,
};
use mohona_daemon::{serve, Call, Client, DaemonInfo, Response, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(17) % 253) as u8).collect()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// An HTTP tracker that tells everyone about one peer.
async fn tracker(peer_port: u16) -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let mut body = b"d8:intervali30e5:peers6:".to_vec();
                body.extend_from_slice(&[127, 0, 0, 1]);
                body.extend_from_slice(&peer_port.to_be_bytes());
                body.push(b'e');
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            });
        }
    });
    port
}

async fn start(dir: &Path) -> (Client, Arc<Notify>) {
    let store = Store::open(&dir.join("test.db")).unwrap();
    let client = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    let manager = Manager::start(
        store,
        client,
        ManagerConfig {
            adaptive: false,
            ..ManagerConfig::default()
        },
    )
    .unwrap();
    let info_path = dir.join("daemon.json");
    let downloads = dir.join("dl");
    std::fs::create_dir_all(&downloads).unwrap();
    let shutdown = Arc::new(Notify::new());
    tokio::spawn(serve(
        manager,
        ServerConfig {
            info_path: info_path.clone(),
            default_dir: downloads,
            stream_program: None,
            shutdown: shutdown.clone(),
        },
    ));
    let started = Instant::now();
    while !DaemonInfo::read(&info_path).is_ok_and(|i| i.pid == std::process::id()) {
        assert!(started.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (Client::from_info_file(&info_path).unwrap(), shutdown)
}

fn add(url: String) -> Call {
    Call::Add {
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
    }
}

async fn status(client: &Client, id: i64) -> (String, Option<String>, u64, Option<String>) {
    let Response::Downloads { downloads } = client.call(Call::List).await.unwrap() else {
        panic!()
    };
    let d = downloads.into_iter().find(|d| d.id == id).unwrap();
    (d.status, d.error, d.downloaded, d.path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_magnet_link_is_downloaded_paused_and_resumed_through_the_daemon() {
    if std::net::UdpSocket::bind("[::]:0").is_err()
        && std::env::var_os("MOHONA_FORCE_TORRENT_TESTS").is_none()
    {
        eprintln!("skipped: this computer has no IPv6 sockets");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("seed");
    std::fs::create_dir_all(&origin).unwrap();
    let file = origin.join("episode.bin");
    let content = data(900_000);
    std::fs::write(&file, &content).unwrap();

    // The seeder.
    let created = create_torrent(
        &file,
        CreateTorrentOptions {
            piece_length: Some(32 * 1024),
            ..CreateTorrentOptions::default()
        },
        &librqbit::spawn_utils::BlockingSpawner::new(1),
    )
    .await
    .unwrap();
    let seed_port = free_port();
    let seeder = Session::new_with_opts(
        dir.path().join("seed-state"),
        SessionOptions {
            dht: None,
            persistence: None,
            ipv4_only: true,
            listen: Some(ListenerOptions {
                listen_addr: ([127, 0, 0, 1], seed_port).into(),
                ipv4_only: true,
                ..ListenerOptions::default()
            }),
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    seeder
        .add_torrent(
            AddTorrent::from_bytes(created.as_bytes().unwrap()),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(origin.display().to_string()),
                ..AddTorrentOptions::default()
            }),
        )
        .await
        .unwrap();
    let tracker_port = tracker(seed_port).await;
    let magnet = format!(
        "magnet:?xt=urn:btih:{}&dn=episode.bin&tr=http%3A%2F%2F127.0.0.1%3A{tracker_port}%2Fannounce",
        created.info_hash().as_string()
    );

    let (client, shutdown) = start(dir.path()).await;
    let Response::Added { id } = client.call(add(magnet.clone())).await.unwrap() else {
        panic!("not added")
    };
    let started = Instant::now();
    let path = loop {
        let (state, error, _, path) = status(&client, id).await;
        assert_ne!(state, "failed", "{error:?}");
        if state == "completed" {
            break path.unwrap();
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "did not finish: {state}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(path.ends_with("episode.bin"), "{path}");
    assert_eq!(std::fs::read(&path).unwrap(), content);

    // A magnet link that nobody serves waits, and can be paused and removed.
    let nobody = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=ghost.bin";
    let Response::Added { id: ghost } = client.call(add(nobody.into())).await.unwrap() else {
        panic!()
    };
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (state, _, _, _) = status(&client, ghost).await;
    assert!(state == "probing" || state == "downloading", "{state}");
    assert!(matches!(
        client.call(Call::Pause { id: ghost }).await.unwrap(),
        Response::Done
    ));
    let started = Instant::now();
    loop {
        let (state, _, _, _) = status(&client, ghost).await;
        if state == "paused" {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{state}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(matches!(
        client
            .call(Call::Remove {
                id: ghost,
                delete_file: false
            })
            .await
            .unwrap(),
        Response::Done
    ));
    shutdown.notify_one();
}
