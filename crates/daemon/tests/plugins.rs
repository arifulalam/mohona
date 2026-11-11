//! A plugin turns a link of its site into the real address.
// The stand-in programs are shell scripts: the tests that use them run on Unix only, so on
// Windows their helpers are unused.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::{serve, Call, Client, DaemonInfo, Response, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use mohona_testserver::{bind, serve as serve_files, Config};
use tokio::sync::Notify;

async fn files() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve_files(
            listener,
            Config {
                default_size: 50_000,
            },
        )
        .await;
    });
    addr
}

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
            cookies: Some("session=secret".into()),
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

async fn url_of(client: &Client, id: i64) -> String {
    match client.call(Call::List).await.unwrap() {
        Response::Downloads { downloads } => {
            downloads.into_iter().find(|d| d.id == id).unwrap().url
        }
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_plugin_gives_the_real_address_but_only_when_switched_on() {
    let dir = tempfile::tempdir().unwrap();
    let server = files().await;
    let plugin = dir.path().join("plugins").join("demo");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(
        plugin.join("plugin.json"),
        r#"{"name":"Demo","version":"1","description":"test","hosts":["plugin.example"],"command":["sh","resolve.sh"]}"#,
    )
    .unwrap();
    // The plugin ignores the link's path and answers with a file of the local test server.
    std::fs::write(
        plugin.join("resolve.sh"),
        format!("#!/bin/sh\ncase \"$1\" in *fail*) echo nope >&2; exit 3;; esac\necho '{{\"url\":\"http://{server}/file\",\"referrer\":\"http://plugin.example/\",\"headers\":{{\"X-Demo\":\"1\"}}}}'\n"),
    )
    .unwrap();
    let (client, shutdown) = start(dir.path()).await;

    // Found, but off.
    match client.call(Call::GetPlugins).await.unwrap() {
        Response::Plugins { plugins, .. } => {
            assert_eq!(plugins.len(), 1);
            assert!(!plugins[0].enabled && plugins[0].problem.is_none());
        }
        other => panic!("{other:?}"),
    }
    // Off: the link is added as it is (and never run).
    let Response::Added { id } = add(&client, "http://plugin.example/page/1").await else {
        panic!()
    };
    assert_eq!(url_of(&client, id).await, "http://plugin.example/page/1");
    let _ = client.call(Call::Cancel { id }).await;

    // On: the real address is downloaded.
    assert!(matches!(
        client
            .call(Call::SetPlugin {
                name: "Demo".into(),
                enabled: true
            })
            .await
            .unwrap(),
        Response::Done
    ));
    let Response::Added { id } = add(&client, "http://plugin.example/page/2").await else {
        panic!()
    };
    assert_eq!(url_of(&client, id).await, format!("http://{server}/file"));
    let started = Instant::now();
    loop {
        match client.call(Call::List).await.unwrap() {
            Response::Downloads { downloads } => {
                if downloads
                    .iter()
                    .any(|d| d.id == id && d.status == "completed")
                {
                    break;
                }
            }
            other => panic!("{other:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "did not finish"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // A plugin that fails stops the add with its name in the message; other sites are untouched.
    match add(&client, "http://plugin.example/fail").await {
        Response::Error { message } => assert!(
            message.contains("Demo") && message.contains("error"),
            "{message}"
        ),
        other => panic!("{other:?}"),
    }
    let Response::Added { id } = add(&client, "http://other.example/x").await else {
        panic!()
    };
    assert_eq!(url_of(&client, id).await, "http://other.example/x");
    // Unknown names are refused.
    assert!(matches!(
        client
            .call(Call::SetPlugin {
                name: "Nope".into(),
                enabled: true
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    shutdown.notify_one();
}
