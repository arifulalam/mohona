//! `mohona scan`: show or set how finished downloads are scanned for viruses (the daemon must be running).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use mohona_daemon::{Call, Client, Response};

#[derive(Args)]
pub struct ScanArgs {
    /// Do not scan finished downloads
    #[arg(long, conflicts_with_all = ["defender", "command"])]
    off: bool,
    /// Scan with Windows Defender (Windows only)
    #[arg(long, conflicts_with = "command")]
    defender: bool,
    /// Scan with your own program; `{file}` stands for the downloaded file (added at the end if missing)
    #[arg(long)]
    command: Option<String>,
    /// The daemon's connection file (default: your application-data folder)
    #[arg(long)]
    info: Option<PathBuf>,
}

async fn run_inner(args: ScanArgs) -> Result<(), String> {
    let path = args.info.unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    let call = if args.off {
        Call::SetScan {
            mode: "off".into(),
            command: String::new(),
        }
    } else if args.defender {
        Call::SetScan {
            mode: "defender".into(),
            command: String::new(),
        }
    } else if let Some(command) = args.command {
        Call::SetScan {
            mode: "custom".into(),
            command,
        }
    } else {
        Call::GetScan
    };
    match client.call(call).await.map_err(|e| e.to_string())? {
        Response::Scan {
            mode,
            command,
            defender,
        } => {
            match mode.as_str() {
                "defender" => println!("Finished downloads are scanned with Windows Defender."),
                "custom" => println!("Finished downloads are scanned with: {command}"),
                _ => println!("Finished downloads are not scanned."),
            }
            match defender {
                Some(path) => println!("Windows Defender is at {path}"),
                None if mode == "defender" => {
                    println!("Windows Defender was not found on this computer.")
                }
                None => {}
            }
            Ok(())
        }
        Response::Error { message } => Err(message),
        _ => Err("the daemon sent an unexpected answer".into()),
    }
}

pub async fn run(args: ScanArgs) -> ExitCode {
    match run_inner(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
