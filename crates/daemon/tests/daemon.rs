use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::{serve, Call, Client, ClientError, DaemonInfo, Response, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use mohona_testserver::{bind, serve as serve_files, Config};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

async fn files() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve_files(
            listener,
            Config {
                default_size: 200_000,
            },
        )
        .await;
    });
    addr
}

struct Daemon {
    client: Client,
    info_path: std::path::PathBuf,
    shutdown: Arc<Notify>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
    downloads: std::path::PathBuf,
}

async fn start(dir: &Path) -> Daemon {
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
    let config = ServerConfig {
        info_path: info_path.clone(),
        default_dir: downloads.clone(),
        stream_program: None,
        shutdown: shutdown.clone(),
    };
    let task = tokio::spawn(serve(manager, config));
    // Wait for the info file.
    let started = Instant::now();
    while !DaemonInfo::read(&info_path).is_ok_and(|i| i.pid == std::process::id()) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "daemon did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Daemon {
        client: Client::from_info_file(&info_path).unwrap(),
        info_path,
        shutdown,
        task,
        downloads,
    }
}

fn add(url: &str) -> Call {
    add_with(url, None, None)
}

fn add_with(url: &str, cookies: Option<&str>, duplicates: Option<&str>) -> Call {
    Call::Add {
        url: url.into(),
        dest_dir: None,
        cookies: cookies.map(Into::into),
        referrer: None,
        user_agent: None,
        headers: vec![],
        sort: false,
        duplicates: duplicates.map(Into::into),
    }
}

async fn wait_for_status(client: &Client, id: i64, status: &str) -> String {
    let started = Instant::now();
    loop {
        let Response::Downloads { downloads } = client.call(Call::List).await.unwrap() else {
            panic!("expected a list");
        };
        let row = downloads.into_iter().find(|d| d.id == id).unwrap();
        if row.status == status || row.status == "failed" {
            assert_eq!(row.status, status, "{:?}", row.error);
            return row.path.unwrap_or_default();
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "stuck at {}",
            row.status
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn answers_ping_downloads_a_file_and_stops_cleanly() {
    let server = files().await;
    let dir = tempfile::tempdir().unwrap();
    let d = start(dir.path()).await;

    let Response::Pong { version, pid } = d.client.call(Call::Ping).await.unwrap() else {
        panic!("expected a pong");
    };
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    assert_eq!(pid, std::process::id());

    let Response::Added { id } = d
        .client
        .call(add(&format!("http://{server}/file?size=150000")))
        .await
        .unwrap()
    else {
        panic!("expected Added");
    };
    let path = wait_for_status(&d.client, id, "completed").await;
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 150_000);
    assert!(Path::new(&path).starts_with(&d.downloads));

    // A second add of the same address is refused unless the client says otherwise.
    let url = format!("http://{server}/file?size=150000");
    match d.client.call(add(&url)).await.unwrap() {
        Response::Error { message } => assert!(message.contains("already"), "{message}"),
        other => panic!("{other:?}"),
    }
    let skip = add_with(&url, None, Some("skip"));
    assert!(matches!(
        d.client.call(skip).await.unwrap(),
        Response::Skipped { .. }
    ));
    let bad = add_with(&url, None, Some("maybe"));
    assert!(matches!(
        d.client.call(bad).await.unwrap(),
        Response::Error { .. }
    ));

    assert_eq!(d.client.call(Call::Shutdown).await.unwrap(), Response::Done);
    d.task.await.unwrap().unwrap();
    assert!(!d.info_path.exists(), "the info file is removed on exit");
    assert!(matches!(
        d.client.call(Call::Ping).await,
        Err(ClientError::NotRunning)
    ));
}

#[tokio::test]
async fn the_browsers_cookie_and_referrer_reach_the_server() {
    let server = files().await;
    let dir = tempfile::tempdir().unwrap();
    let d = start(dir.path()).await;
    // Without the cookie the server refuses, so the download fails.
    let Response::Added { id } = d
        .client
        .call(add(&format!("http://{server}/cookie?size=1000")))
        .await
        .unwrap()
    else {
        panic!()
    };
    let started = Instant::now();
    loop {
        let Response::Downloads { downloads } = d.client.call(Call::List).await.unwrap() else {
            panic!()
        };
        if downloads.iter().any(|x| x.id == id && x.status == "failed") {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // With it, it works.
    let with_cookie = add_with(
        &format!("http://{server}/cookie?size=2000"),
        Some("session=ok"),
        Some("rename"),
    );
    let Response::Added { id } = d.client.call(with_cookie).await.unwrap() else {
        panic!()
    };
    wait_for_status(&d.client, id, "completed").await;
    d.shutdown.notify_one();
    d.task.await.unwrap().unwrap();
}

#[tokio::test]
async fn pause_resume_and_unknown_ids() {
    let dir = tempfile::tempdir().unwrap();
    let d = start(dir.path()).await;
    for call in [
        Call::Pause { id: 99 },
        Call::Resume { id: 99 },
        Call::Cancel { id: 99 },
        Call::Remove {
            id: 99,
            delete_file: false,
        },
    ] {
        match d.client.call(call).await.unwrap() {
            Response::Error { message } => assert!(message.contains("99"), "{message}"),
            other => panic!("{other:?}"),
        }
    }
    d.shutdown.notify_one();
    d.task.await.unwrap().unwrap();
}

#[tokio::test]
async fn strangers_get_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let d = start(dir.path()).await;
    let info = DaemonInfo::read(&d.info_path).unwrap();
    let addr = SocketAddr::from(([127, 0, 0, 1], info.port));

    // A wrong token: the connection is closed without an answer.
    let mut wrong = info.clone();
    wrong.token = "0".repeat(info.token.len());
    let result = Client::new(&wrong).call(Call::Ping).await;
    assert!(
        matches!(result, Err(ClientError::Protocol(_))),
        "{result:?}"
    );

    // A web page speaking HTTP to the port gets nothing either.
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /?token=x HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut got = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert!(got.is_empty());

    // An enormous line is dropped.
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let junk = vec![b'a'; 2 * 1024 * 1024];
    let _ = stream.write_all(&junk).await;
    let mut got = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut got)).await;
    assert!(got.is_empty());

    // The right token still works afterwards.
    assert!(matches!(
        Client::new(&info).call(Call::Ping).await.unwrap(),
        Response::Pong { .. }
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&d.info_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the info file holds a secret: {mode:o}");
    }
    d.shutdown.notify_one();
    d.task.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_second_daemon_is_refused_but_a_stale_info_file_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let d = start(dir.path()).await;

    let store = Store::open(&dir.path().join("other.db")).unwrap();
    let client = HttpClient::new(&ClientConfig::default()).unwrap();
    let manager = Manager::start(store, client, ManagerConfig::default()).unwrap();
    let result = serve(
        manager,
        ServerConfig {
            info_path: d.info_path.clone(),
            default_dir: dir.path().to_path_buf(),
            stream_program: None,
            shutdown: Arc::new(Notify::new()),
        },
    )
    .await;
    assert!(result.unwrap_err().to_string().contains("already running"));

    d.shutdown.notify_one();
    d.task.await.unwrap().unwrap();

    // A leftover file from a crash (nobody answers) does not block a new start.
    DaemonInfo {
        port: 9,
        token: "old".into(),
        pid: 1,
        version: "0".into(),
    }
    .write(&d.info_path)
    .unwrap();
    let again = start(dir.path()).await;
    assert!(matches!(
        again.client.call(Call::Ping).await.unwrap(),
        Response::Pong { .. }
    ));
    again.shutdown.notify_one();
    again.task.await.unwrap().unwrap();
}

async fn page_link(server: SocketAddr, size: u64) -> String {
    let mut stream = TcpStream::connect(server).await.unwrap();
    stream
        .write_all(
            format!(
                "GET /expiring/page?size={size} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut text = String::new();
    stream.read_to_string(&mut text).await.unwrap();
    let start = text.find("href=\"").unwrap() + 6;
    let end = start + text[start..].find('"').unwrap();
    format!("http://{server}{}", text[start..end].replace("&amp;", "&"))
}

#[tokio::test]
async fn a_failed_expired_download_says_so_and_can_continue_from_a_fresh_link() {
    let server = files().await;
    let dir = tempfile::tempdir().unwrap();
    let d = start(dir.path()).await;
    let page = format!("http://{server}/expiring/page?size=40000");
    let old = page_link(server, 40_000).await;
    // The page is loaded again: the first link is stale.
    page_link(server, 40_000).await;

    let Response::Added { id } = d
        .client
        .call(Call::Add {
            url: old,
            dest_dir: None,
            cookies: None,
            referrer: Some(page.clone()),
            user_agent: None,
            headers: vec![],
            sort: false,
            duplicates: None,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    // Wait until it fails, then look at what the list says.
    let started = Instant::now();
    let row = loop {
        let Response::Downloads { downloads } = d.client.call(Call::List).await.unwrap() else {
            panic!()
        };
        let row = downloads.into_iter().find(|x| x.id == id).unwrap();
        if row.status == "failed" {
            break row;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{}",
            row.status
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(row.expired, "{row:?}");
    assert_eq!(row.page.as_deref(), Some(page.as_str()));

    // A link that does not work is refused and changes nothing.
    let refused = d
        .client
        .call(Call::Refresh {
            id,
            url: format!("http://{server}/expiring/file?size=40000&token=99"),
            cookies: None,
            referrer: None,
            user_agent: None,
        })
        .await
        .unwrap();
    assert!(matches!(refused, Response::Error { .. }), "{refused:?}");
    let fresh = page_link(server, 40_000).await;
    let answer = d
        .client
        .call(Call::Refresh {
            id,
            url: fresh,
            cookies: Some("a=b".into()),
            referrer: None,
            user_agent: None,
        })
        .await
        .unwrap();
    assert_eq!(answer, Response::Done);
    let path = wait_for_status(&d.client, id, "completed").await;
    assert_eq!(std::fs::metadata(path).unwrap().len(), 40_000);

    // Nothing to refresh on a finished download, or an unknown one.
    for id in [id, 77] {
        let answer = d
            .client
            .call(Call::Refresh {
                id,
                url: "http://x/y".into(),
                cookies: None,
                referrer: None,
                user_agent: None,
            })
            .await
            .unwrap();
        assert!(matches!(answer, Response::Error { .. }), "{answer:?}");
    }
    d.shutdown.notify_one();
    d.task.await.unwrap().unwrap();
}

#[tokio::test]
async fn clients_keep_extension_settings_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let d = start(dir.path()).await;
    let get = |key: &str| Call::SettingGet { key: key.into() };
    let set = |key: &str, value: &str| Call::SettingSet {
        key: key.into(),
        value: value.into(),
    };
    assert!(matches!(
        d.client.call(get("extension.settings")).await.unwrap(),
        Response::Setting { value: None }
    ));
    assert!(matches!(
        d.client
            .call(set("extension.settings", "{}"))
            .await
            .unwrap(),
        Response::Done
    ));
    match d.client.call(get("extension.settings")).await.unwrap() {
        Response::Setting { value } => assert_eq!(value.as_deref(), Some("{}")),
        other => panic!("{other:?}"),
    }
    for key in ["theme", "../x", "extension.A", ""] {
        assert!(matches!(
            d.client.call(set(key, "x")).await.unwrap(),
            Response::Error { .. }
        ));
    }
    let big = "x".repeat(70_000);
    assert!(matches!(
        d.client
            .call(set("extension.settings", &big))
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    d.shutdown.notify_one();
    d.task.await.unwrap().unwrap();
}

#[tokio::test]
async fn the_app_can_read_and_change_the_engines_settings() {
    let dir = tempfile::tempdir().unwrap();
    let server = files().await;
    let d = start(dir.path()).await;
    let set = |folder: Option<&str>, speed: Option<u64>, active: Option<u32>| Call::SetConfig {
        download_folder: folder.map(str::to_string),
        speed_limit: speed,
        max_active: active,
    };

    let first = d.client.call(Call::GetConfig).await.unwrap();
    let Response::Config {
        download_folder,
        default_folder,
        speed_limit,
        max_active,
    } = first
    else {
        panic!("{first:?}")
    };
    assert_eq!(download_folder, default_folder);
    assert_eq!(speed_limit, None);
    assert!(max_active >= 1);

    // Refused values change nothing, and say why.
    for bad in [
        set(Some("relative/folder"), Some(1000), None),
        set(None, Some(5000), Some(0)),
        set(None, None, Some(99)),
    ] {
        let Response::Error { message } = d.client.call(bad).await.unwrap() else {
            panic!("should be refused")
        };
        assert!(!message.is_empty());
    }
    let Response::Config { speed_limit, .. } = d.client.call(Call::GetConfig).await.unwrap() else {
        panic!()
    };
    assert_eq!(
        speed_limit, None,
        "a refused change must not be half applied"
    );

    // A good change is kept, and new downloads use the new folder.
    let chosen = dir.path().join("my downloads");
    let Response::Config {
        download_folder,
        speed_limit,
        max_active,
        ..
    } = d
        .client
        .call(set(Some(chosen.to_str().unwrap()), Some(500_000), Some(2)))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(download_folder, chosen.display().to_string());
    assert_eq!((speed_limit, max_active), (Some(500_000), 2));
    assert!(chosen.is_dir(), "the folder is created");

    let url = format!("http://{server}/file?size=20000&seed=2");
    let Response::Added { id } = d.client.call(add(&url)).await.unwrap() else {
        panic!()
    };
    let started = Instant::now();
    loop {
        let Response::Downloads { downloads } = d.client.call(Call::List).await.unwrap() else {
            panic!()
        };
        let row = downloads.iter().find(|r| r.id == id).unwrap();
        if row.status == "completed" {
            assert!(
                row.path
                    .as_deref()
                    .unwrap()
                    .starts_with(chosen.to_str().unwrap()),
                "{:?}",
                row.path
            );
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{row:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 0 removes the limit; an empty folder goes back to the default.
    let Response::Config {
        download_folder,
        speed_limit,
        ..
    } = d.client.call(set(Some(""), Some(0), None)).await.unwrap()
    else {
        panic!()
    };
    assert_eq!(speed_limit, None);
    assert_eq!(download_folder, default_folder);
    d.shutdown.notify_one();
    d.task.await.unwrap().unwrap();
}
