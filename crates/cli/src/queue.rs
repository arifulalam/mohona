//! `mohona queue ...`: the download list that is kept in the database.

use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;
use mohona_manager::{DownloadStatus, Manager, ManagerConfig, ManagerEvent};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;

use crate::{describe, format_bytes, parse_rate};

#[derive(Subcommand)]
pub enum QueueCommand {
    /// Add downloads to the list (they start with `queue run`)
    Add {
        /// Addresses to download
        #[arg(required = true)]
        urls: Vec<String>,
        /// Folder to save the files in
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
    },
    /// Show the list
    List,
    /// Run the list until everything is done (Ctrl-C stops and keeps progress)
    Run {
        /// How many downloads may run at once
        #[arg(short, long)]
        simultaneous: Option<u32>,
        /// Speed limit for everything together, like 1M, or `off`. It is saved.
        #[arg(short = 'l', long, value_parser = parse_rate)]
        max_speed: Option<u64>,
    },
    /// Limit the speed of one download, like `queue speed 3 500K` (or `off`)
    Speed {
        /// The number shown by `queue list`
        id: i64,
        /// The limit, like 500K or 1.5M, or `off`
        #[arg(value_parser = parse_rate)]
        limit: u64,
    },
    /// Put a paused, failed or canceled download back in the list to run
    Retry {
        /// The number shown by `queue list`
        id: i64,
    },
    /// Take a download off the list
    Remove {
        /// The number shown by `queue list`
        id: i64,
    },
}

pub async fn run(command: QueueCommand, db: PathBuf) -> Result<(), String> {
    let store = Store::open(&db).map_err(|e| format!("cannot open {}: {e}", db.display()))?;
    match command {
        QueueCommand::Add { urls, output } => {
            let dir = output.to_string_lossy().into_owned();
            for url in urls {
                let id = store
                    .add_download(url.trim(), &dir, None)
                    .map_err(|e| e.to_string())?;
                println!("#{id} added: {url}");
            }
            Ok(())
        }
        QueueCommand::List => list(&store),
        QueueCommand::Retry { id } => {
            let row = store
                .download(id)
                .map_err(|e| e.to_string())?
                .ok_or(format!("no download #{id}"))?;
            if matches!(
                row.status,
                DownloadStatus::Paused | DownloadStatus::Failed | DownloadStatus::Canceled
            ) {
                store
                    .set_status(id, DownloadStatus::Queued, None)
                    .map_err(|e| e.to_string())?;
                println!("#{id} will run next time");
                Ok(())
            } else {
                Err(format!("#{id} is {}", row.status.as_str()))
            }
        }
        QueueCommand::Remove { id } => {
            store
                .download(id)
                .map_err(|e| e.to_string())?
                .ok_or(format!("no download #{id}"))?;
            store.delete_download(id).map_err(|e| e.to_string())
        }
        QueueCommand::Speed { id, limit } => {
            store
                .download(id)
                .map_err(|e| e.to_string())?
                .ok_or(format!("no download #{id}"))?;
            store
                .set_download_speed_limit(id, (limit > 0).then_some(limit))
                .map_err(|e| e.to_string())
        }
        QueueCommand::Run {
            simultaneous,
            max_speed,
        } => {
            if let Some(n) = simultaneous {
                store
                    .set_queue_max_active(1, n)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(rate) = max_speed {
                if rate > 0 {
                    store
                        .set_setting("speed_limit", &rate.to_string())
                        .map_err(|e| e.to_string())?;
                } else {
                    store
                        .remove_setting("speed_limit")
                        .map_err(|e| e.to_string())?;
                }
            }
            run_queue(store).await
        }
    }
}

fn list(store: &Store) -> Result<(), String> {
    let rows = store.list_downloads().map_err(|e| e.to_string())?;
    if rows.is_empty() {
        println!("The list is empty. Add downloads with `mohona queue add <url>`.");
    }
    for row in rows {
        let name = row.filename.as_deref().unwrap_or(&row.url);
        let progress = match row.size {
            Some(total) if total > 0 => format!(
                "{} of {}",
                format_bytes(row.downloaded),
                format_bytes(total)
            ),
            _ => format_bytes(row.downloaded),
        };
        print!(
            "#{:<3} {:<11} {:<24} {}",
            row.id,
            row.status.as_str(),
            progress,
            name
        );
        if let Some(error) = row.error {
            print!("  ({error})");
        }
        println!();
    }
    Ok(())
}

async fn run_queue(store: Store) -> Result<(), String> {
    let client = HttpClient::new(&ClientConfig::default()).map_err(|e| describe(&e))?;
    let manager =
        Manager::start(store, client, ManagerConfig::default()).map_err(|e| e.to_string())?;
    let mut events = manager.subscribe();

    let stopper = manager.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nstopping, progress is kept...");
            stopper.shutdown().await;
        }
    });

    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            event = events.recv() => {
                if let Ok(ManagerEvent::State { id, status }) = event {
                    announce(&manager, id, status).await;
                }
            }
            _ = tick.tick() => {
                let Ok(rows) = manager.list().await else {
                    return Ok(()); // shut down by Ctrl-C
                };
                let waiting = rows.iter().any(|r| {
                    matches!(r.status, DownloadStatus::Queued) || r.status.is_active()
                });
                if !waiting {
                    let failed = rows.iter().filter(|r| r.status == DownloadStatus::Failed).count();
                    manager.shutdown().await;
                    return if failed > 0 {
                        Err(format!("{failed} download(s) failed; see `mohona queue list`"))
                    } else {
                        Ok(())
                    };
                }
            }
        }
    }
}

async fn announce(manager: &Manager, id: i64, status: DownloadStatus) {
    let Ok(rows) = manager.list().await else {
        return;
    };
    let Some(row) = rows.iter().find(|r| r.id == id) else {
        return;
    };
    let name = row.filename.as_deref().unwrap_or(&row.url);
    match status {
        DownloadStatus::Downloading => println!("#{id} downloading {name}"),
        DownloadStatus::Completed => {
            println!("#{id} done: {}", row.path.as_deref().unwrap_or(name))
        }
        DownloadStatus::Failed => println!(
            "#{id} failed: {}",
            row.error.as_deref().unwrap_or("unknown error")
        ),
        _ => {}
    }
}
