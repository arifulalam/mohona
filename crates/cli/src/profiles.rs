//! `mohona profiles ...`: how to talk to particular sites (the daemon must be running).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mohona_daemon::profiles::Profile;
use mohona_daemon::{Call, Client, Response};

#[derive(Subcommand)]
pub enum ProfilesCommand {
    /// List the profiles (the first enabled one for a site is used)
    List,
    /// Add a profile, like `profiles add "Slow site" --host files.example.com --connections 2 --speed 500K`
    Add {
        /// A name to recognise it by
        name: String,
        /// The site: example.com (and its subdomains), *.example.com, or a pattern with *
        #[arg(long)]
        host: String,
        /// Most connections one download from this site may use (1 to 32)
        #[arg(long)]
        connections: Option<u32>,
        /// Speed limit for each download from this site, like 500K or 2M
        #[arg(long)]
        speed: Option<String>,
        #[arg(long)]
        user_agent: Option<String>,
        #[arg(long)]
        referrer: Option<String>,
        /// An extra header as "Name: value" (repeat for more; never cookies or logins)
        #[arg(long = "header")]
        headers: Vec<String>,
    },
    /// Remove a profile by its number
    Remove { id: i64 },
    /// Switch a profile on
    Enable { id: i64 },
    /// Switch a profile off (it stays in the list)
    Disable { id: i64 },
}

fn client(info: Option<PathBuf>) -> Result<Client, String> {
    let path = info.unwrap_or_else(mohona_daemon::default_info_path);
    Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())
}

async fn call(client: &Client, call: Call) -> Result<Response, String> {
    match client.call(call).await.map_err(|e| e.to_string())? {
        Response::Error { message } => Err(message),
        other => Ok(other),
    }
}

async fn list(client: &Client) -> Result<Vec<Profile>, String> {
    match call(client, Call::GetProfiles).await? {
        Response::Profiles { profiles } => Ok(profiles),
        _ => Err("the daemon sent an unexpected answer".into()),
    }
}

fn describe(p: &Profile) -> String {
    let mut set = Vec::new();
    if let Some(c) = p.connections {
        set.push(format!("at most {c} connections"));
    }
    if let Some(s) = p.speed_limit {
        set.push(format!("{} per download", crate::format_bytes(s) + "/s"));
    }
    if let Some(u) = &p.user_agent {
        set.push(format!("user agent {u:?}"));
    }
    if let Some(r) = &p.referrer {
        set.push(format!("referrer {r}"));
    }
    for (n, v) in &p.headers {
        set.push(format!("header {n}: {v}"));
    }
    format!(
        "{:>3}  {}  {}  ({})\n       {}",
        p.id,
        if p.enabled { "on " } else { "off" },
        p.name,
        p.host,
        set.join(", ")
    )
}

async fn run_inner(command: ProfilesCommand, info: Option<PathBuf>) -> Result<(), String> {
    let client = client(info)?;
    let mut profiles = list(&client).await?;
    match command {
        ProfilesCommand::List => {
            if profiles.is_empty() {
                println!("No profiles yet. Add one with: mohona profiles add NAME --host example.com --connections 2");
            }
            for p in &profiles {
                println!("{}", describe(p));
            }
            return Ok(());
        }
        ProfilesCommand::Add {
            name,
            host,
            connections,
            speed,
            user_agent,
            referrer,
            headers,
        } => {
            let speed_limit = match speed {
                Some(text) => Some(crate::parse_rate(&text)?).filter(|s| *s > 0),
                None => None,
            };
            let mut pairs = Vec::new();
            for h in headers {
                let (n, v) = h
                    .split_once(':')
                    .ok_or_else(|| format!("a header is written as \"Name: value\", not {h:?}"))?;
                pairs.push((n.trim().to_string(), v.trim().to_string()));
            }
            profiles.push(Profile {
                id: 0,
                name,
                enabled: true,
                host,
                connections,
                speed_limit,
                user_agent,
                referrer,
                headers: pairs,
            });
        }
        ProfilesCommand::Remove { id } => {
            let before = profiles.len();
            profiles.retain(|p| p.id != id);
            if profiles.len() == before {
                return Err(format!("no profile with number {id}"));
            }
        }
        ProfilesCommand::Enable { id } | ProfilesCommand::Disable { id } => {
            let on = matches!(command, ProfilesCommand::Enable { .. });
            let Some(p) = profiles.iter_mut().find(|p| p.id == id) else {
                return Err(format!("no profile with number {id}"));
            };
            p.enabled = on;
        }
    }
    call(&client, Call::SetProfiles { profiles }).await?;
    println!("Saved.");
    Ok(())
}

pub async fn run(command: ProfilesCommand, info: Option<PathBuf>) -> ExitCode {
    match run_inner(command, info).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
