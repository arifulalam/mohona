//! Video pages (YouTube and alike) are downloaded by yt-dlp, here a stand-in script.

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
