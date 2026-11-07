//! `mohona update`: look for a newer version on GitHub Releases (the daemon must be running).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Args;
use mohona_daemon::{Call, Client, Response};

#[derive(Args)]
pub struct UpdateArgs {
    /// Look in this GitHub repository (owner/name); remembered. Empty (`--repo ""`) turns the check off.
    #[arg(long)]
    repo: Option<String>,
    /// Only show which repository is set; do not ask GitHub
    #[arg(long)]
    show: bool,
    /// The daemon's connection file (default: your application-data folder)
    #[arg(long)]
    info: Option<PathBuf>,
}

async fn run_inner(args: UpdateArgs) -> Result<(), String> {
    let cleared = args.repo.as_deref().is_some_and(|r| r.trim().is_empty());
    let path = args.info.unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    if let Some(repo) = args.repo {
        if let Response::Error { message } = client
            .call(Call::SetUpdateRepo { repo })
            .await
            .map_err(|e| e.to_string())?
        {
            return Err(message);
        }
    }
    let call = if args.show || cleared {
        Call::GetUpdate
    } else {
        Call::CheckUpdate
    };
    match client.call(call).await.map_err(|e| e.to_string())? {
        Response::Update {
            repo,
            current,
            latest,
            newer,
            url,
            notes,
        } => {
            if repo.is_empty() {
                println!("No repository is set. Use: mohona update --repo owner/name");
                return Ok(());
            }
            println!("Mohona {current}; looking in {repo}.");
            match (latest, newer) {
                (None, _) => {}
                (Some(v), true) => {
                    println!("A newer version is out: {v}");
                    if let Some(url) = url {
                        println!("Get it at {url}");
                    }
                    if let Some(notes) = notes.filter(|n| !n.trim().is_empty()) {
                        println!("\n{}", notes.trim());
                    }
                }
                (Some(v), false) => {
                    println!("You have the newest version (the latest release is {v}).")
                }
            }
            Ok(())
        }
        Response::Error { message } => Err(message),
        _ => Err("the daemon sent an unexpected answer".into()),
    }
}

pub async fn run(args: UpdateArgs) -> ExitCode {
    match run_inner(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
