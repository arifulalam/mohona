//! Video pages (YouTube and alike) are downloaded by yt-dlp, here a stand-in script.
// The stand-in programs are shell scripts: the tests that use them run on Unix only, so on
// Windows their helpers are unused.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::{serve, Call, Client, DaemonInfo, Response, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
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

async fn add(client: &Client, url: &str) -> Response {
    client
        .call(Call::Add {
            url: url.into(),
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
            choice: None,
        })
        .await
        .unwrap()
}

async fn row(client: &Client, id: i64) -> mohona_daemon::DownloadInfo {
    match client.call(Call::List).await.unwrap() {
        Response::Downloads { downloads } => downloads.into_iter().find(|d| d.id == id).unwrap(),
        other => panic!("{other:?}"),
    }
}

async fn until(client: &Client, id: i64, status: &str) -> mohona_daemon::DownloadInfo {
    let started = Instant::now();
    loop {
        let r = row(client, id).await;
        if r.status == status {
            return r;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "stuck at {} ({:?})",
            r.status,
            r.error
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
fn script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_video_page_is_downloaded_through_yt_dlp_and_can_be_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("fake-yt-dlp");
    // Like yt-dlp: options, then `--`, then the address. The first line it is given decides what it does.
    script(
        &tool,
        r#"#!/bin/sh
folder=""; prev=""
for a in "$@"; do
  if [ "$prev" = "-P" ]; then folder="$a"; fi
  prev="$a"
  url="$a"
done
echo "$@" > "$folder/args.txt"
case "$url" in
  *failing*) echo "ERROR: [youtube] failing: Video unavailable" >&2; exit 1;;
  *slow*) echo "MOHONA_NAME Slow one"; echo "MOHONA_PROGRESS 100 1000 NA"; sleep 30; exit 0;;
esac
echo "MOHONA_NAME Fake video"
echo "MOHONA_PROGRESS 0 4000 NA"
echo "MOHONA_PROGRESS 4000 4000 NA"
echo "MOHONA_PROGRESS 500 1000 NA"
head -c 1500 /dev/zero > "$folder/Fake video [abc].mp4"
echo "MOHONA_FILE $folder/Fake video [abc].mp4"
"#,
    );
    let (client, shutdown) = start(dir.path()).await;
    assert!(matches!(
        client
            .call(Call::SetTool {
                name: "yt-dlp".into(),
                path: Some(tool.display().to_string())
            })
            .await
            .unwrap(),
        Response::Tools { .. } | Response::Done
    ));

    // It finishes as an ordinary download: a row, a size, a file.
    let Response::Added { id } = add(&client, "https://www.youtube.com/watch?v=abc12345678").await
    else {
        panic!()
    };
    let done = until(&client, id, "completed").await;
    let path = done.path.unwrap();
    assert!(path.ends_with("Fake video [abc].mp4"), "{path}");
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 1500);
    assert_eq!(done.filename.as_deref(), Some("Fake video"));
    let args = std::fs::read_to_string(dir.path().join("dl").join("args.txt")).unwrap();
    assert!(
        args.trim_end()
            .ends_with("-- https://www.youtube.com/watch?v=abc12345678"),
        "{args}"
    );

    // A failure shows yt-dlp's own words.
    let Response::Added { id } = add(&client, "https://www.youtube.com/watch?v=failing0001").await
    else {
        panic!()
    };
    let failed = until(&client, id, "failed").await;
    assert!(failed.error.unwrap().contains("Video unavailable"));

    // Pausing stops the program.
    let Response::Added { id } = add(&client, "https://www.youtube.com/watch?v=slow0000001").await
    else {
        panic!()
    };
    until(&client, id, "downloading").await;
    client.call(Call::Cancel { id }).await.unwrap();
    until(&client, id, "canceled").await;

    // Without yt-dlp the message says what to install and where to get it. (The chosen program is
    // gone: a choice is never second-guessed by looking elsewhere.)
    std::fs::remove_file(&tool).unwrap();
    let Response::Added { id } = add(&client, "https://youtu.be/missing00001").await else {
        panic!()
    };
    let failed = until(&client, id, "failed").await;
    let message = failed.error.unwrap();
    assert!(
        message.contains("yt-dlp was not found")
            && message.contains("winget install yt-dlp.yt-dlp")
            && message.contains("github.com/yt-dlp"),
        "{message}"
    );
    shutdown.notify_one();
}

#[cfg(unix)]
#[tokio::test]
async fn other_pages_are_tried_with_yt_dlp_then_downloaded_normally_and_the_browser_is_used() {
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("fake-yt-dlp");
    script(
        &tool,
        r#"#!/bin/sh
folder=""; prev=""
for a in "$@"; do
  if [ "$prev" = "-P" ]; then folder="$a"; fi
  prev="$a"
  url="$a"
done
echo "$@" > "$folder/args.txt"
case "$url" in
  *"/file?"*) echo "ERROR: Unsupported URL: $url" >&2; exit 1;;
esac
echo "MOHONA_NAME A reel"
echo "MOHONA_PROGRESS 10 100 NA"
head -c 700 /dev/zero > "$folder/A reel.mp4"
echo "MOHONA_FILE $folder/A reel.mp4"
"#,
    );
    // A plain file server for the fallback (on this computer, so local addresses must count here).
    std::env::set_var("MOHONA_VIDEO_GUESS_LOCAL", "1");
    let listener = mohona_testserver::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let server = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ =
            mohona_testserver::serve(listener, mohona_testserver::Config { default_size: 3000 })
                .await;
    });
    let (client, shutdown) = start(dir.path()).await;
    client
        .call(Call::SetTool {
            name: "yt-dlp".into(),
            path: Some(tool.display().to_string()),
        })
        .await
        .unwrap();
    let args_file = dir.path().join("dl").join("args.txt");

    // yt-dlp does not know the page: it is downloaded as an ordinary file, 3000 bytes.
    let Response::Added { id } = add(&client, &format!("http://{server}/file?size=3000")).await
    else {
        panic!()
    };
    let done = until(&client, id, "completed").await;
    assert_eq!(std::fs::metadata(done.path.unwrap()).unwrap().len(), 3000);
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert!(args.contains("--use-extractors default,-generic"), "{args}");

    // A page yt-dlp knows is taken by yt-dlp, with the chosen browser's sign-in.
    assert!(matches!(
        client
            .call(Call::SetVideoBrowser {
                browser: Some("--exec=calc".into())
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    client
        .call(Call::SetVideoBrowser {
            browser: Some("Edge".into()),
        })
        .await
        .unwrap();
    assert!(matches!(
        client.call(Call::GetVideoBrowser).await.unwrap(),
        Response::Setting { value: Some(v) } if v == "edge"
    ));
    let Response::Added { id } = add(&client, "https://www.instagram.com/reel/abc123/").await
    else {
        panic!()
    };
    let done = until(&client, id, "completed").await;
    assert_eq!(done.filename.as_deref(), Some("A reel"));
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert!(args.contains("--cookies-from-browser edge"), "{args}");

    // Asked for the video of a page that has none: an error, not a saved web page.
    let Response::Added { id } = client
        .call(Call::Add {
            url: format!("http://{server}/file?size=3000&again=1"),
            dest_dir: None,
            cookies: Some("a=b".into()),
            referrer: None,
            user_agent: None,
            headers: vec![],
            sort: false,
            duplicates: None,
            queue: None,
            files: None,
            video: true,
            choice: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let failed = until(&client, id, "failed").await;
    assert!(
        failed.error.unwrap().contains("no video on this page"),
        "the page has no video"
    );
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert!(args.contains("--cookies "), "{args}");
    assert!(!args.contains("Cookie:"), "{args}");
    assert!(!args.contains("--use-extractors"), "{args}");
    shutdown.notify_one();
}

#[tokio::test]
async fn the_sharing_of_finished_torrents_is_kept_and_checked() {
    let dir = tempfile::tempdir().unwrap();
    let (client, shutdown) = start(dir.path()).await;
    assert!(matches!(
        client.call(Call::GetSeeding).await.unwrap(),
        Response::Seeding {
            ratio: None,
            minutes: None
        }
    ));
    assert!(matches!(
        client
            .call(Call::SetSeeding {
                ratio: Some(150),
                minutes: Some(90)
            })
            .await
            .unwrap(),
        Response::Seeding {
            ratio: Some(150),
            minutes: Some(90)
        }
    ));
    assert!(matches!(
        client
            .call(Call::SetSeeding {
                ratio: Some(u32::MAX),
                minutes: None
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    // Zero or nothing turns it off.
    assert!(matches!(
        client
            .call(Call::SetSeeding {
                ratio: Some(0),
                minutes: None
            })
            .await
            .unwrap(),
        Response::Seeding {
            ratio: None,
            minutes: None
        }
    ));
    shutdown.notify_one();
}

#[cfg(unix)]
#[tokio::test]
async fn the_chosen_quality_and_format_reach_yt_dlp() {
    let dir = tempfile::tempdir().unwrap();
    let tool = dir.path().join("fake-yt-dlp");
    script(
        &tool,
        r#"#!/bin/sh
folder=""; prev=""
for a in "$@"; do
  if [ "$prev" = "-P" ]; then folder="$a"; fi
  prev="$a"
done
echo "$@" > "$folder/args.txt"
echo "MOHONA_NAME Song"
head -c 800 /dev/zero > "$folder/Song.mp3"
echo "MOHONA_FILE $folder/Song.mp3"
"#,
    );
    let (client, shutdown) = start(dir.path()).await;
    client
        .call(Call::SetTool {
            name: "yt-dlp".into(),
            path: Some(tool.display().to_string()),
        })
        .await
        .unwrap();
    let read = |r: Response| match r {
        Response::VideoFormat {
            quality,
            container,
            audio,
        } => (quality, container, audio),
        other => panic!("{other:?}"),
    };
    // The default: the best, as mp4 (and mp3 for sound).
    assert_eq!(
        read(client.call(Call::GetVideoFormat).await.unwrap()),
        ("best".to_string(), "mp4".to_string(), "mp3".to_string())
    );
    // Only what the program knows is accepted, and nothing is changed by a refusal.
    for (quality, container, audio) in [
        ("--exec=calc", "mp4", "mp3"),
        ("720", "exe", "mp3"),
        ("720", "mp4", "mp3 --x"),
    ] {
        assert!(matches!(
            client
                .call(Call::SetVideoFormat {
                    quality: quality.into(),
                    container: container.into(),
                    audio: audio.into()
                })
                .await
                .unwrap(),
            Response::Error { .. }
        ));
    }
    assert_eq!(
        read(client.call(Call::GetVideoFormat).await.unwrap()).0,
        "best"
    );

    let args_file = dir.path().join("dl").join("args.txt");
    let set = |quality: &str, container: &str, audio: &str| Call::SetVideoFormat {
        quality: quality.into(),
        container: container.into(),
        audio: audio.into(),
    };
    client.call(set("720", "mkv", "mp3")).await.unwrap();
    let Response::Added { id } = add(&client, "https://www.youtube.com/watch?v=abc12345678").await
    else {
        panic!()
    };
    until(&client, id, "completed").await;
    let args = std::fs::read_to_string(&args_file).unwrap();
    // (No ffmpeg is found here, so only the height limit applies.)
    assert!(
        args.contains("-f b[height<=720]") || args.contains("bv*[height<=720]"),
        "{args}"
    );
    assert!(!args.contains("--exec"), "{args}");

    client.call(set("audio", "mp4", "mp3")).await.unwrap();
    let Response::Added { id } = add(&client, "https://www.youtube.com/watch?v=abc12345679").await
    else {
        panic!()
    };
    until(&client, id, "completed").await;
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert!(args.contains("-f ba/b"), "{args}");

    // A choice made for one download beats the saved one (which is still "sound only").
    let Response::Added { id } = client
        .call(Call::Add {
            url: "https://www.youtube.com/watch?v=abc12345680".into(),
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
            choice: Some(mohona_net::VideoChoice {
                quality: "480".into(),
                container: "mp4".into(),
                audio: "mp3".into(),
            }),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    until(&client, id, "completed").await;
    let args = std::fs::read_to_string(&args_file).unwrap();
    assert!(args.contains("height<=480"), "{args}");
    shutdown.notify_one();
}
