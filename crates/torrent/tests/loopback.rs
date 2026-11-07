//! A real transfer between two sessions on this computer: one seeds a file, Mohona's engine downloads it.

use std::time::{Duration, Instant};

use librqbit::{
    create_torrent, AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session,
    SessionOptions,
};
use mohona_core::CancelToken;
use mohona_torrent::{Seeding, Source, TorrentError, Torrents};

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
    // A folder's files are looked for in the output folder itself; a file's, next to it.
    let parent = if path.is_dir() {
        path
    } else {
        path.parent().unwrap()
    }
    .display()
    .to_string();
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
            None,
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
            None,
            &CancelToken::new(),
            |_| {},
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(again.path, done.path);

    // A torrent nobody has can be stopped. (Not the seeded one: the engine may find a seeder on this
    // computer by itself, e.g. through local peer discovery on Windows.)
    let other = origin.join("nobody.bin");
    std::fs::write(
        &other,
        data(300_000).iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
    )
    .unwrap();
    let unseeded = create_torrent(
        &other,
        CreateTorrentOptions {
            piece_length: Some(32 * 1024),
            ..CreateTorrentOptions::default()
        },
        &librqbit::spawn_utils::BlockingSpawner::new(1),
    )
    .await
    .unwrap()
    .as_bytes()
    .unwrap()
    .to_vec();
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
            Source::File(unseeded),
            &dir.path().join("out2"),
            vec![],
            None,
            &cancel,
            |_| {},
        )
        .await;
    assert!(matches!(result, Err(TorrentError::Cancelled)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_chosen_files_of_a_torrent_are_downloaded() {
    if std::net::UdpSocket::bind("[::]:0").is_err()
        && std::env::var_os("MOHONA_FORCE_TORRENT_TESTS").is_none()
    {
        eprintln!("skipped: this computer has no IPv6 sockets");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("seed").join("album");
    std::fs::create_dir_all(&folder).unwrap();
    let parts = [
        ("a.bin", 200_000usize),
        ("b.bin", 150_000),
        ("c.bin", 120_000),
    ];
    for (i, (name, len)) in parts.iter().enumerate() {
        let bytes: Vec<u8> = data(*len).iter().map(|b| b.wrapping_add(i as u8)).collect();
        std::fs::write(folder.join(name), bytes).unwrap();
    }
    let (torrent, peer, _seeder) = seed(&folder, &dir.path().join("seed-state")).await;

    let torrents = Torrents::new(dir.path().join("state"));
    // The listing names every file with its size, without downloading anything.
    let listing = torrents
        .list(
            Source::File(torrent.clone()),
            vec![peer],
            &CancelToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(listing.name, "album");
    let mut seen: Vec<(String, u64)> = listing
        .files
        .iter()
        .map(|f| (f.path.rsplit('/').next().unwrap().to_string(), f.size))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        parts
            .iter()
            .map(|(n, l)| (n.to_string(), *l as u64))
            .collect::<Vec<_>>()
    );
    let wanted = listing
        .files
        .iter()
        .find(|f| f.path.ends_with("b.bin"))
        .unwrap()
        .index;

    let out = dir.path().join("out");
    let last = std::sync::Mutex::new(None);
    let result = tokio::time::timeout(
        Duration::from_secs(40),
        torrents.run(
            Source::File(torrent),
            &out,
            vec![peer],
            Some(vec![wanted]),
            &CancelToken::new(),
            |u| *last.lock().unwrap() = Some(u),
        ),
    )
    .await;
    let result = result
        .unwrap_or_else(|_| panic!("did not finish; last report: {:?}", last.lock().unwrap()));
    result.expect("and succeeds");
    let got = out.join("album").join("b.bin");
    assert_eq!(std::fs::metadata(&got).unwrap().len(), 150_000);
    // The files that were not chosen are absent or empty, except the few bytes that share a
    // piece with the chosen file (a piece is 32 KiB).
    let chosen = std::fs::read(&got).unwrap();
    assert_eq!(
        chosen[..100],
        data(150_000)[..100]
            .iter()
            .map(|b| b.wrapping_add(1))
            .collect::<Vec<_>>()[..]
    );
    for n in ["a.bin", "c.bin"] {
        let on_disk = out.join("album").join(n);
        if on_disk.exists() {
            let filled = std::fs::read(&on_disk)
                .unwrap()
                .iter()
                .filter(|b| **b != 0)
                .count();
            assert!(filled <= 32 * 1024, "{n} was downloaded ({filled} bytes)");
        }
    }
    // Nothing spilled into the download folder itself.
    assert!(!out.join("b.bin").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finished_torrent_is_shared_until_sharing_is_turned_off() {
    if std::net::UdpSocket::bind("[::]:0").is_err()
        && std::env::var_os("MOHONA_FORCE_TORRENT_TESTS").is_none()
    {
        eprintln!("skipped: this computer has no IPv6 sockets");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("seed");
    std::fs::create_dir_all(&origin).unwrap();
    let file = origin.join("share.bin");
    std::fs::write(&file, data(200_000)).unwrap();
    let (torrent, peer, _seeder) = seed(&file, &dir.path().join("seed-state")).await;

    let torrents = Torrents::new(dir.path().join("state"));
    // Not shared by default: the torrent leaves the session when it is done.
    torrents
        .run(
            Source::File(torrent.clone()),
            &dir.path().join("a"),
            vec![peer],
            None,
            &CancelToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(torrents.active().await, 0);

    // Shared for a long time: it stays in the session after it is done ...
    torrents.set_seeding(Seeding {
        ratio: Some(100.0),
        minutes: Some(600),
    });
    torrents
        .run(
            Source::File(torrent.clone()),
            &dir.path().join("b"),
            vec![peer],
            None,
            &CancelToken::new(),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(torrents.active().await, 1);

    // ... until sharing is turned off.
    torrents.set_seeding(Seeding::default());
    let started = Instant::now();
    while torrents.active().await != 0 {
        assert!(started.elapsed() < Duration::from_secs(20), "still shared");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
