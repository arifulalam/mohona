//! `mohona queue ...`: the download list that is kept in the database.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use clap::Subcommand;
use mohona_manager::{
    find_duplicate, parse_days, AddRequest, AfterAction, DownloadStatus, DuplicatePolicy, Manager,
    ManagerConfig, ManagerEvent, Moment, Schedule, Window,
};
use mohona_net::{ClientConfig, HttpClient};
use mohona_store::Store;

use crate::{describe, format_bytes, parse_rate};

#[derive(Subcommand)]
pub enum CategoryCommand {
    /// Add a category, like `queue categories add Video mp4,mkv,webm --folder Videos`
    Add {
        name: String,
        /// File extensions, comma separated, without dots
        extensions: String,
        /// Folder: absolute, or inside the download folder (default: the name)
        #[arg(long)]
        folder: Option<String>,
    },
    /// Remove a category by its number
    Remove { id: i64 },
}

fn parse_policy(text: &str) -> Result<DuplicatePolicy, String> {
    match text {
        "ask" => Ok(DuplicatePolicy::Ask),
        "rename" => Ok(DuplicatePolicy::Rename),
        "skip" => Ok(DuplicatePolicy::Skip),
        "overwrite" => Ok(DuplicatePolicy::Overwrite),
        other => Err(format!(
            "unknown policy {other:?}: use ask, rename, skip or overwrite"
        )),
    }
}

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
        /// Move finished files into the folder of their category
        #[arg(long)]
        sort: bool,
        /// What to do about duplicates: ask, rename, skip or overwrite
        #[arg(long, default_value = "ask", value_parser = parse_policy)]
        duplicates: DuplicatePolicy,
    },
    /// Show or change the categories files are sorted into
    Categories {
        #[command(subcommand)]
        action: Option<CategoryCommand>,
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
        /// What to do when everything is done: exit, sleep, hibernate,
        /// shutdown or run:<program> [arguments]. A countdown runs first;
        /// Ctrl-C during it cancels the action.
        #[arg(long, value_parser = AfterAction::parse)]
        then: Option<AfterAction>,
        /// Seconds of countdown before the action
        #[arg(long, default_value_t = 60)]
        countdown: u32,
    },
    /// Choose when the queue may run, like
    /// `queue schedule --days mon-fri --from 01:00 --to 07:00 --speed 2M`.
    /// With no options it shows the schedule.
    Schedule {
        /// Days the window opens on: all, mon-fri, sat,sun, mon,wed-fri ...
        #[arg(long, default_value = "all")]
        days: String,
        /// Opening time, like 01:00
        #[arg(long, requires = "to")]
        from: Option<String>,
        /// Closing time, like 07:00 (earlier than --from means after midnight)
        #[arg(long, requires = "from")]
        to: Option<String>,
        /// Speed limit while the window is open, like 2M
        #[arg(long, value_parser = parse_rate)]
        speed: Option<u64>,
        /// Add the window to the schedule instead of replacing it
        #[arg(long)]
        add: bool,
        /// Remove the schedule: the queue may run at any time
        #[arg(long, conflicts_with_all = ["from", "to"])]
        clear: bool,
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
        QueueCommand::Add {
            urls,
            output,
            sort,
            duplicates,
        } => {
            let dir = output.to_string_lossy().into_owned();
            let mut refused = 0;
            for url in urls {
                let mut request = AddRequest::new(url.trim(), output.clone());
                request.sort_into_categories = sort;
                request.duplicates = duplicates;
                if let Some(found) = find_duplicate(&store, &request) {
                    match duplicates {
                        DuplicatePolicy::Ask => {
                            println!("{url}: {found}. Use --duplicates rename, skip or overwrite.");
                            refused += 1;
                            continue;
                        }
                        DuplicatePolicy::Skip => {
                            println!("{url}: skipped, {found}");
                            continue;
                        }
                        DuplicatePolicy::Overwrite => match &found {
                            mohona_manager::Duplicate::SameAddress { id, path, .. } => {
                                if let Some(path) = path {
                                    let _ = std::fs::remove_file(path);
                                }
                                store.delete_download(*id).map_err(|e| e.to_string())?;
                            }
                            mohona_manager::Duplicate::FileExists { path } => {
                                let _ = std::fs::remove_file(path);
                            }
                        },
                        DuplicatePolicy::Rename => {}
                    }
                }
                let id = store
                    .add_download_with(url.trim(), &dir, None, sort)
                    .map_err(|e| e.to_string())?;
                let _ = store.add_log(id, "info", "added");
                println!("#{id} added: {url}");
            }
            if refused > 0 {
                Err(format!("{refused} not added"))
            } else {
                Ok(())
            }
        }
        QueueCommand::Categories { action } => categories(&store, action),
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
        QueueCommand::Schedule {
            days,
            from,
            to,
            speed,
            add,
            clear,
        } => schedule(&store, days, from, to, speed, add, clear),
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
            then,
            countdown,
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
            run_queue(store, then.map(|action| (action, countdown))).await
        }
    }
}

fn describe_schedule(schedule: &Schedule) -> String {
    schedule
        .windows
        .iter()
        .map(|w| {
            let days = if w.days.is_empty() {
                "every day".to_string()
            } else {
                w.days.join(",")
            };
            let speed = w.speed_limit.map_or(String::new(), |n| {
                format!(", at most {}/s", format_bytes(n))
            });
            format!("{days} {}-{}{speed}", w.start, w.end)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn schedule(
    store: &Store,
    days: String,
    from: Option<String>,
    to: Option<String>,
    speed: Option<u64>,
    add: bool,
    clear: bool,
) -> Result<(), String> {
    const QUEUE: i64 = 1;
    let queue = store
        .queues()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|q| q.id == QUEUE)
        .ok_or("no queue")?;
    let current = queue
        .schedule_json
        .as_deref()
        .and_then(|json| Schedule::from_json(json).ok());
    if clear {
        store
            .set_queue_schedule(QUEUE, None)
            .map_err(|e| e.to_string())?;
        println!("The queue may run at any time.");
        return Ok(());
    }
    if let (Some(start), Some(end)) = (from, to) {
        let window = Window {
            days: parse_days(&days)?,
            start,
            end,
            speed_limit: speed.filter(|n| *n > 0),
        };
        let mut windows = if add {
            current.map(|s| s.windows).unwrap_or_default()
        } else {
            Vec::new()
        };
        windows.push(window);
        let schedule = Schedule { windows };
        schedule.check()?;
        store
            .set_queue_schedule(QUEUE, Some(&schedule.to_json()))
            .map_err(|e| e.to_string())?;
        println!("Schedule: {}", describe_schedule(&schedule));
        return Ok(());
    }
    match current {
        Some(schedule) => {
            println!("Schedule: {}", describe_schedule(&schedule));
            report_window(&schedule);
        }
        None => println!("The queue may run at any time."),
    }
    Ok(())
}

/// Say whether the schedule is open now and when that changes.
fn report_window(schedule: &Schedule) {
    let now = Moment::now();
    let open = schedule.open_window(now).is_some();
    let change = schedule.minutes_to_change(now).map(|minutes| {
        if minutes >= 120 {
            format!("{} h {} min", minutes / 60, minutes % 60)
        } else {
            format!("{minutes} min")
        }
    });
    match (open, change) {
        (true, Some(c)) => println!("Open now; closes in {c}."),
        (true, None) => println!("Open now."),
        (false, Some(c)) => println!("Closed now; opens in {c}."),
        (false, None) => println!("Closed."),
    }
}

fn categories(store: &Store, action: Option<CategoryCommand>) -> Result<(), String> {
    match action {
        None => {
            let all = store.categories().map_err(|e| e.to_string())?;
            if all.is_empty() {
                println!("No categories. Add one with `mohona queue categories add`.");
            }
            for c in all {
                println!(
                    "#{} {} -> {}  ({})",
                    c.id,
                    c.name,
                    c.folder.as_deref().unwrap_or(&c.name),
                    c.extensions.join(", ")
                );
            }
            Ok(())
        }
        Some(CategoryCommand::Add {
            name,
            extensions,
            folder,
        }) => {
            let list: Vec<String> = extensions
                .split(',')
                .map(|e| e.trim().trim_start_matches('.').to_lowercase())
                .filter(|e| !e.is_empty())
                .collect();
            let id = store
                .add_category(&name, &list, folder.as_deref())
                .map_err(|e| e.to_string())?;
            println!("#{id} {name} added");
            Ok(())
        }
        Some(CategoryCommand::Remove { id }) => {
            store.remove_category(id).map_err(|e| e.to_string())?;
            println!("#{id} removed");
            Ok(())
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

async fn run_queue(store: Store, then: Option<(AfterAction, u32)>) -> Result<(), String> {
    let client = HttpClient::new(&ClientConfig::default()).map_err(|e| describe(&e))?;
    let manager =
        Manager::start(store, client, ManagerConfig::default()).map_err(|e| e.to_string())?;
    let mut events = manager.subscribe();
    if let Some(schedule) = manager
        .queues()
        .await
        .ok()
        .and_then(|queues| queues.into_iter().next())
        .and_then(|q| q.schedule_json)
        .and_then(|json| Schedule::from_json(&json).ok())
    {
        report_window(&schedule);
    }

    // The action waits for downloads that finish in this run.
    let mut armed = then.is_some();
    if let Some((action, seconds)) = then {
        manager
            .set_after_action(Some(action.clone()), seconds)
            .await
            .map_err(|e| e.to_string())?;
        println!(
            "When everything is done: {} (after a {seconds} s countdown).",
            action.describe()
        );
    }
    let counting = Arc::new(AtomicBool::new(false));
    let mut finished_here = 0u32;

    let stopper = manager.clone();
    let counting_for_signal = counting.clone();
    tokio::spawn(async move {
        while tokio::signal::ctrl_c().await.is_ok() {
            if counting_for_signal.load(Ordering::SeqCst) {
                eprintln!("\ncancelling the action");
                let _ = stopper.cancel_action().await;
            } else {
                eprintln!("\nstopping, progress is kept...");
                stopper.shutdown().await;
                break;
            }
        }
    });

    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Ok(ManagerEvent::State { id, status }) => {
                        if status == DownloadStatus::Completed {
                            finished_here += 1;
                        }
                        announce(&manager, id, status).await;
                    }
                    Ok(ManagerEvent::ActionCountdown { action, seconds_left }) => {
                        counting.store(true, Ordering::SeqCst);
                        if seconds_left % 10 == 0 || seconds_left <= 10 {
                            println!(
                                "{} in {seconds_left} s - press Ctrl-C to cancel",
                                capitalize(&action.describe())
                            );
                        }
                    }
                    Ok(ManagerEvent::ActionCancelled { .. }) => {
                        counting.store(false, Ordering::SeqCst);
                        armed = false;
                        println!("The action was cancelled.");
                    }
                    Ok(ManagerEvent::ActionDone { action }) => {
                        counting.store(false, Ordering::SeqCst);
                        armed = false;
                        println!("Done: {}.", action.describe());
                    }
                    Ok(ManagerEvent::ActionFailed { message }) => {
                        counting.store(false, Ordering::SeqCst);
                        armed = false;
                        eprintln!("The action failed: {message}");
                    }
                    Ok(ManagerEvent::QueueWindow { open, .. }) => {
                        println!("{}", if open { "The schedule opened." } else { "The schedule closed; downloads are stopped until it opens." });
                    }
                    Ok(ManagerEvent::NetworkChanged { online }) => {
                        println!(
                            "{}",
                            if online {
                                "The network changed; downloads are paused until it is steady."
                            } else {
                                "No network; downloads are paused until it is back."
                            }
                        );
                    }
                    Ok(ManagerEvent::NetworkResumed) => {
                        println!("The network is back; continuing downloads.");
                    }
                    Ok(ManagerEvent::RetryScheduled { id, attempt, wait, message }) => {
                        println!(
                            "#{id} network problem ({message}); trying again in {} s (retry {attempt})",
                            wait.as_secs()
                        );
                    }
                    _ => {}
                }
            }
            _ = tick.tick() => {
                let Ok(rows) = manager.list().await else {
                    return Ok(()); // shut down by Ctrl-C
                };
                let waiting = rows.iter().any(|r| {
                    matches!(r.status, DownloadStatus::Queued) || r.status.is_active()
                });
                // With an action armed, stay until it has happened (or was
                // cancelled), but only if something finished in this run.
                let waiting_for_action = armed && (finished_here > 0 || counting.load(Ordering::SeqCst));
                if !waiting && !waiting_for_action {
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

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
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
