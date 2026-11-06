//! `mohona tool ...`: the outside programs Mohona can use (ffmpeg, yt-dlp), where they are, and where to get them.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mohona_daemon::{Call, Client, Response};

#[derive(Subcommand)]
pub enum ToolCommand {
    /// Show each tool: what it is for, where it is, and where to download it
    Status,
    /// Say where a tool is (`ffmpeg` or `yt-dlp`; the file, or the folder that holds it)
    Set { name: String, path: PathBuf },
    /// Go back to finding a tool by itself
    Clear { name: String },
}

async fn run_inner(command: ToolCommand, info: Option<PathBuf>) -> Result<(), String> {
    let path = info.unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    let call = match command {
        ToolCommand::Status => Call::GetTools,
        ToolCommand::Set { name, path } => Call::SetTool {
            name,
            path: Some(path.display().to_string()),
        },
        ToolCommand::Clear { name } => Call::SetTool { name, path: None },
    };
    let was_status = matches!(call, Call::GetTools);
    match client.call(call).await.map_err(|e| e.to_string())? {
        Response::Error { message } => Err(message),
        Response::Tools { tools } if was_status => {
            for t in tools {
                println!("{}", t.name);
                println!("  Needed to: {}", t.purpose);
                match (&t.found, &t.chosen) {
                    (Some(f), Some(_)) => println!("  Using: {f} (your choice)"),
                    (Some(f), None) => println!("  Using: {f} (found by itself)"),
                    (None, Some(c)) => println!("  NOT FOUND at the path you chose: {c}"),
                    (None, None) => println!("  NOT FOUND"),
                }
                if t.found.is_none() {
                    println!(
                        "  Windows: {}   or download from {}",
                        t.winget, t.windows_page
                    );
                    println!("  Other systems: {}", t.page);
                }
            }
            Ok(())
        }
        _ => {
            println!("Done. Check with: mohona tool status");
            Ok(())
        }
    }
}

pub async fn run(command: ToolCommand, info: Option<PathBuf>) -> ExitCode {
    match run_inner(command, info).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
