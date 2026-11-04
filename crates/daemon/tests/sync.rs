//! Two computers (two daemons) keeping settings the same through one shared folder.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mohona_daemon::{serve, Call, Client, DaemonInfo, Response, ServerConfig};
use mohona_manager::{Manager, ManagerConfig};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;
use tokio::sync::Notify;

struct Pc {
    client: Client,
    shutdown: Arc<Notify>,
}

async fn start(dir: &Path) -> Pc {
    std::fs::create_dir_all(dir).unwrap();
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
                return Pc {
                    client: Client::new(&info),
                    shutdown,
                };
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "daemon did not start"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn sync(pc: &Pc, folder: &Path) -> (Vec<String>, Vec<String>) {
    match pc
        .client
        .call(Call::SyncNow {
            folder: Some(folder.display().to_string()),
        })
        .await
        .unwrap()
    {
        Response::Synced { applied, wrote } => (applied, wrote),
        other => panic!("{other:?}"),
    }
}

async fn rules(pc: &Pc) -> Vec<mohona_daemon::rules::Rule> {
    match pc.client.call(Call::GetRules).await.unwrap() {
        Response::Rules { rules } => rules,
        other => panic!("{other:?}"),
    }
}

fn rule(name: &str, enabled: bool) -> mohona_daemon::rules::Rule {
    serde_json::from_value(serde_json::json!({
        "name": name, "enabled": enabled,
        "when": { "extensions": ["iso"] }, "then": { "sort": true }
    }))
    .unwrap()
}

#[tokio::test]
async fn settings_follow_a_person_between_two_computers_through_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("cloud");
    std::fs::create_dir_all(&shared).unwrap();
    let a = start(&dir.path().join("a")).await;
    let b = start(&dir.path().join("b")).await;

    // Computer A has a rule and shares it.
    let r = a
        .client
        .call(Call::SetRules {
            rules: vec![rule("ISOs", true)],
        })
        .await
        .unwrap();
    assert!(matches!(r, Response::Done), "{r:?}");
    let (applied, wrote) = sync(&a, &shared).await;
    assert!(
        applied.is_empty() && wrote == vec!["rules.list".to_string()],
        "{applied:?} {wrote:?}"
    );
    assert!(shared.join("mohona-settings.json").exists());
    // Never anything that belongs to one computer.
    let text = std::fs::read_to_string(shared.join("mohona-settings.json")).unwrap();
    assert!(
        !text.contains("api.tokens") && !text.contains("download_folder"),
        "{text}"
    );

    // Computer B takes it over, and a second sync changes nothing.
    let (applied, wrote) = sync(&b, &shared).await;
    assert_eq!(applied, vec!["rules.list".to_string()]);
    assert!(wrote.is_empty());
    assert_eq!(rules(&b).await[0].name, "ISOs");
    assert_eq!(sync(&b, &shared).await, (vec![], vec![]));

    // B changes it later; A follows.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let mut changed = rules(&b).await;
    changed[0].enabled = false;
    b.client
        .call(Call::SetRules { rules: changed })
        .await
        .unwrap();
    let (_, wrote) = sync(&b, &shared).await;
    assert_eq!(wrote, vec!["rules.list".to_string()]);
    let (applied, _) = sync(&a, &shared).await;
    assert_eq!(applied, vec!["rules.list".to_string()]);
    assert!(!rules(&a).await[0].enabled);

    // A damaged file is reported and nothing is touched.
    std::fs::write(shared.join("mohona-settings.json"), "{ not json").unwrap();
    match a
        .client
        .call(Call::SyncNow {
            folder: Some(shared.display().to_string()),
        })
        .await
        .unwrap()
    {
        Response::Error { message } => assert!(message.contains("damaged"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(!rules(&a).await[0].enabled);
    // So is a folder that is not there.
    match a
        .client
        .call(Call::SetSync {
            folder: Some(dir.path().join("nope").display().to_string()),
        })
        .await
        .unwrap()
    {
        Response::Error { .. } => {}
        other => panic!("{other:?}"),
    }
    a.shutdown.notify_one();
    b.shutdown.notify_one();
}
