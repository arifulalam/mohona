//! `mohona torrent ...`: look inside a torrent and download only some of its files (the daemon must be running).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mohona_daemon::{Call, Client, Response};

#[derive(Subcommand)]
pub enum TorrentCommand {
    /// List the files of a torrent (a magnet link can take up to 90 seconds to answer)
    Files { url: String },
    /// Add a torrent and download only the files with these numbers (from `files`), like `--files 0,2,5`
    Add {
        url: String,
        #[arg(long, value_delimiter = ',', required = true)]
        files: Vec<u32>,
        /// Save in this folder (default: the download folder)
        #[arg(long)]
        folder: Option<String>,
    },
    /// Show or set how long finished torrents are shared. Without options: show. `--off` stops sharing.
    Seed {
        /// Stop at this upload ratio (for example 1.5)
        #[arg(long)]
        ratio: Option<f64>,
        /// Stop after this many minutes
        #[arg(long)]
        minutes: Option<u64>,
        /// Do not share finished torrents
        #[arg(long, conflicts_with_all = ["ratio", "minutes"])]
        off: bool,
    },
    /// Show or set a speed limit for all torrents together, in KB/s. Without options: show.
    Speed {
        /// Download at most this many KB/s (0 for no limit)
        #[arg(long)]
        down: Option<u32>,
        /// Upload at most this many KB/s (0 for no limit)
        #[arg(long)]
        up: Option<u32>,
        /// No limit, either way
        #[arg(long, conflicts_with_all = ["down", "up"])]
        off: bool,
    },
}

async fn run_inner(command: TorrentCommand, info: Option<PathBuf>) -> Result<(), String> {
    let path = info.unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    match command {
        TorrentCommand::Files { url } => {
            match client
                .call(Call::TorrentFiles { url })
                .await
                .map_err(|e| e.to_string())?
            {
                Response::TorrentFiles { name, files } => {
                    println!("{name}");
                    for f in files {
                        println!(
                            "{:>5}  {:>10}  {}",
                            f.index,
                            crate::format_bytes(f.size),
                            f.path
                        );
                    }
                    Ok(())
                }
                Response::Error { message } => Err(message),
                _ => Err("the daemon sent an unexpected answer".into()),
            }
        }
        TorrentCommand::Seed {
            ratio,
            minutes,
            off,
        } => {
            if ratio.is_some_and(|r| !r.is_finite() || !(0.0..=1000.0).contains(&r)) {
                return Err("the ratio must be a number from 0 to 1000".into());
            }
            let call = if off || ratio.is_some() || minutes.is_some() {
                Call::SetSeeding {
                    ratio: ratio.map(|r| (r * 100.0).round() as u32),
                    minutes,
                }
            } else {
                Call::GetSeeding
            };
            match client.call(call).await.map_err(|e| e.to_string())? {
                Response::Seeding { ratio, minutes } => {
                    match (ratio, minutes) {
                        (None, None) => println!("Finished torrents are not shared."),
                        (r, m) => {
                            let mut parts = Vec::new();
                            if let Some(r) = r {
                                parts.push(format!(
                                    "an upload ratio of {:.2}",
                                    f64::from(r) / 100.0
                                ));
                            }
                            if let Some(m) = m {
                                parts.push(format!("{m} minutes"));
                            }
                            println!(
                                "Finished torrents are shared until {} (whichever comes first).",
                                parts.join(" or ")
                            );
                        }
                    }
                    Ok(())
                }
                Response::Error { message } => Err(message),
                _ => Err("the daemon sent an unexpected answer".into()),
            }
        }
        TorrentCommand::Speed { down, up, off } => {
            let ask = |call: Call| async {
                match client.call(call).await.map_err(|e| e.to_string())? {
                    Response::TorrentSpeed { down_kb, up_kb } => Ok((down_kb, up_kb)),
                    Response::Error { message } => Err(message),
                    _ => Err("the daemon sent an unexpected answer".to_string()),
                }
            };
            let (now_down, now_up) = ask(Call::GetTorrentSpeed).await?;
            let (down_kb, up_kb) = if off {
                ask(Call::SetTorrentSpeed {
                    down_kb: None,
                    up_kb: None,
                })
                .await?
            } else if down.is_some() || up.is_some() {
                // What is not mentioned stays as it is.
                ask(Call::SetTorrentSpeed {
                    down_kb: down.map_or(now_down, |k| Some(k).filter(|k| *k > 0)),
                    up_kb: up.map_or(now_up, |k| Some(k).filter(|k| *k > 0)),
                })
                .await?
            } else {
                (now_down, now_up)
            };
            let show = |kb: Option<u32>| kb.map_or("no limit".to_string(), |k| format!("{k} KB/s"));
            println!(
                "Torrents: download {}, upload {}.",
                show(down_kb),
                show(up_kb)
            );
            Ok(())
        }
        TorrentCommand::Add { url, files, folder } => {
            let call = Call::Add {
                url,
                dest_dir: folder,
                cookies: None,
                referrer: None,
                user_agent: None,
                headers: Vec::new(),
                sort: false,
                duplicates: None,
                queue: None,
                files: Some(files),
                video: false,
            };
            match client.call(call).await.map_err(|e| e.to_string())? {
                Response::Added { id } => {
                    println!("Added as download {id}.");
                    Ok(())
                }
                Response::Skipped { reason } => Err(format!("not added: {reason}")),
                Response::Error { message } => Err(message),
                _ => Err("the daemon sent an unexpected answer".into()),
            }
        }
    }
}

pub async fn run(command: TorrentCommand, info: Option<PathBuf>) -> ExitCode {
    match run_inner(command, info).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
