//! `mohona sync ...`: keep portable settings the same on several computers through a folder you already sync.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mohona_daemon::{Call, Client, Response};

#[derive(Subcommand)]
pub enum SyncCommand {
    /// Choose the folder to sync through (OneDrive, Dropbox, Syncthing ...); the daemon syncs it every 5 minutes
    Set { folder: PathBuf },
    /// Stop syncing
    Off,
    /// Sync now (with the chosen folder, or the one given)
    Now { folder: Option<PathBuf> },
    /// Show the chosen folder
    Status,
}

async fn call(client: &Client, call: Call) -> Result<Response, String> {
    match client.call(call).await.map_err(|e| e.to_string())? {
        Response::Error { message } => Err(message),
        other => Ok(other),
    }
}

fn text(path: &std::path::Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
        .trim_start_matches(r"\\?\")
        .to_string()
}

async fn run_inner(command: SyncCommand, info: Option<PathBuf>) -> Result<(), String> {
    let path = info.unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    match command {
        SyncCommand::Set { folder } => {
            let folder = text(&folder);
            call(
                &client,
                Call::SetSync {
                    folder: Some(folder.clone()),
                },
            )
            .await?;
            println!("Syncing through {folder}. Do the same on the other computers, with the same synced folder.");
            now(&client, None).await
        }
        SyncCommand::Off => {
            call(&client, Call::SetSync { folder: None }).await?;
            println!("Sync is off. The file in the folder is left as it is.");
            Ok(())
        }
        SyncCommand::Now { folder } => now(&client, folder.map(|f| text(&f))).await,
        SyncCommand::Status => {
            match call(&client, Call::GetSync).await? {
                Response::SyncFolder { folder: Some(f) } => println!("Syncing through {f}"),
                _ => println!("Sync is off. Choose a folder with: mohona sync set FOLDER"),
            }
            Ok(())
        }
    }
}

async fn now(client: &Client, folder: Option<String>) -> Result<(), String> {
    match call(client, Call::SyncNow { folder }).await? {
        Response::Synced { applied, wrote } => {
            if applied.is_empty() && wrote.is_empty() {
                println!("Already the same everywhere.");
            }
            for a in applied {
                println!("Took over from the folder: {a}");
            }
            for w in wrote {
                println!("Wrote to the folder: {w}");
            }
            Ok(())
        }
        _ => Err("the daemon sent an unexpected answer".into()),
    }
}

pub async fn run(command: SyncCommand, info: Option<PathBuf>) -> ExitCode {
    match run_inner(command, info).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
