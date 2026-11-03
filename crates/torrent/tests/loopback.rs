//! A real transfer between two sessions on this computer: one seeds a file, Mohona's engine downloads it.

use std::time::{Duration, Instant};

use librqbit::{
    create_torrent, AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session,
    SessionOptions,
};
use mohona_core::CancelToken;
use mohona_torrent::{Source, TorrentError, Torrents};

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i.wrapping_mul(31) % 251) as u8).collect()
}

/// Seeds `path` and returns the .torrent file and the seeder's address.
async fn seed(
    path: &std::path::Path,
    state: &std::path::Path,
) -> (Vec<u8>, std::net::SocketAddr, std::sync::Arc<Session>) {
    let created = create_torrent(
        path,
        CreateTorrentOptions {
            piece_length: Some(32 * 1024),
            ..CreateTorrentOptions::default()
        },
        &librqbit::spawn_utils::BlockingSpawner::new(1),
    )
    .await
    .unwrap();
    let bytes = created.as_bytes().unwrap().to_vec();
    // A free port, found first so the seeder's address is known.
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let session = Session::new_with_opts(
        state.to_path_buf(),
        SessionOptions {
            dht: None,
            persistence: None,
            ipv4_only: true,
            listen: Some(ListenerOptions {
                listen_addr: ([127, 0, 0, 1], port).into(),
                ipv4_only: true,
                ..ListenerOptions::default()
            }),
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    let parent = path.parent().unwrap().display().to_string();
    session
        .add_torrent(
            AddTorrent::from_bytes(bytes.clone()),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(parent),
                ..AddTorrentOptions::default()
            }),
        )
        .await
        .unwrap();
    (bytes, ([127, 0, 0, 1], port).into(), session)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_torrent_is_downloaded_from_a_seeder_and_can_be_stopped() {
    // The engine opens an IPv6 socket for trackers; a computer without any (rare) cannot run this.
    if std::net::UdpSocket::bind("[::]:0").is_err()
        && std::env::var_os("MOHONA_FORCE_TORRENT_TESTS").is_none()
    {
        eprintln!("skipped: this computer has no IPv6 sockets");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("seed");
    std::fs::create_dir_all(&origin).unwrap();
    let file = origin.join("movie.bin");
    let content = data(600_000);
    std::fs::write(&file, &content).unwrap();
    let (torrent, peer, _seeder) = seed(&file, &dir.path().join("seed-state")).await;

    let torrents = Torrents::new(dir.path().join("state"));
    let out = dir.path().join("out");
    let mut last = None;
    let done = tokio::time::timeout(
        Duration::from_secs(60),
        torrents.run(
            Source::File(torrent.clone()),
            &out,
            vec![peer],
            &CancelToken::new(),
            |u| last = Some(u),
        ),
    )
    .await
    .expect("the transfer finishes")
    .expect("and succeeds");
    assert_eq!(done.name, "movie.bin");
    assert_eq!(done.bytes, 600_000);
    assert_eq!(std::fs::read(&done.path).unwrap(), content);
    let last = last.unwrap();
    assert_eq!(last.total, Some(600_000));

    // Asked again for the same files, it finds them complete and finishes at once.
    let again = tokio::time::timeout(
        Duration::from_secs(30),
        torrents.run(
            Source::File(torrent.clone()),
            &out,
            vec![peer],
            &CancelToken::new(),
            |_| {},
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(again.path, done.path);

    // A torrent nobody has can be stopped.
    let lonely = Torrents::new(dir.path().join("state2"));
    let cancel = CancelToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        stopper.cancel();
    });
    let started = Instant::now();
    let result = lonely
        .run(
            Source::File(torrent),
            &dir.path().join("out2"),
            vec![],
            &cancel,
            |_| {},
        )
        .await;
    assert!(matches!(result, Err(TorrentError::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}
