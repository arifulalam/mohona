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
