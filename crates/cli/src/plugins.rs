//! `mohona plugins ...`: small programs that teach Mohona about one site (the daemon must be running).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mohona_daemon::{Call, Client, Response};

#[derive(Subcommand)]
pub enum PluginsCommand {
    /// List the plugins found in the plugins folder, and show where it is
    List,
    /// Switch a plugin on (plugins are off until you do)
    Enable { name: String },
    /// Switch a plugin off
    Disable { name: String },
}

async fn run_inner(command: PluginsCommand, info: Option<PathBuf>) -> Result<(), String> {
    let path = info.unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    let (name, enabled) = match command {
        PluginsCommand::List => {
            return match client
                .call(Call::GetPlugins)
                .await
                .map_err(|e| e.to_string())?
            {
                Response::Plugins { folder, plugins } => {
                    println!("Plugins folder: {folder}");
                    if plugins.is_empty() {
                        println!("No plugins. Put a folder with a plugin.json there (see docs/plugins.md).");
                    }
                    for p in plugins {
                        match p.problem {
                            Some(why) => println!("  ??   {}  not usable: {why}", p.name),
                            None => println!(
                                "  {}  {} {}  ({})  {}",
                                if p.enabled { "on " } else { "off" },
                                p.name,
                                p.version,
                                p.hosts.join(", "),
                                p.description
                            ),
                        }
                    }
                    Ok(())
                }
                Response::Error { message } => Err(message),
                _ => Err("the daemon sent an unexpected answer".into()),
            };
        }
        PluginsCommand::Enable { name } => (name, true),
        PluginsCommand::Disable { name } => (name, false),
    };
    match client
        .call(Call::SetPlugin {
            name: name.clone(),
            enabled,
        })
        .await
        .map_err(|e| e.to_string())?
    {
        Response::Error { message } => Err(message),
        _ => {
            println!(
                "{name} is {}.",
                if enabled {
                    "on: it can run programs of its folder when you add links of its sites"
                } else {
                    "off"
                }
            );
            Ok(())
        }
    }
}

pub async fn run(command: PluginsCommand, info: Option<PathBuf>) -> ExitCode {
    match run_inner(command, info).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
