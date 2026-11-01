//! `mohona rules ...`: rules for the running daemon ("when a download matches this, do that").

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mohona_daemon::rules::{Rule, Then, When};
use mohona_daemon::{Call, Client, Response};

#[derive(Subcommand)]
pub enum RulesCommand {
    /// List the rules in the order they are tried (the first match wins)
    List,
    /// Add a rule, like `rules add "Zips" --ext zip,7z --folder Archives`
    Add {
        /// A name to recognise it by
        name: String,
        /// Site: example.com (and its subdomains), *.example.com, or a pattern with *
        #[arg(long)]
        host: Option<String>,
        /// File types, comma separated, without dots
        #[arg(long)]
        ext: Option<String>,
        /// Text the address must contain
        #[arg(long)]
        contains: Option<String>,
        /// Site of the page the link was on
        #[arg(long)]
        referrer_host: Option<String>,
        /// Save in this folder (full path, or a name inside the download folder)
        #[arg(long)]
        folder: Option<String>,
        /// Put it in this queue (its number)
        #[arg(long)]
        queue: Option<i64>,
        /// Move the finished file into its category's folder
        #[arg(long)]
        sort: bool,
        /// Refuse these downloads
        #[arg(long)]
        block: bool,
    },
    /// Remove a rule by its number
    Remove { id: i64 },
    /// Switch a rule on
    Enable { id: i64 },
    /// Switch a rule off (it stays in the list)
    Disable { id: i64 },
    /// Show which rule would apply to an address (nothing is added)
    Test {
        url: String,
        /// The page the link was on
        #[arg(long)]
        referrer: Option<String>,
    },
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

async fn list(client: &Client) -> Result<Vec<Rule>, String> {
    match call(client, Call::GetRules).await? {
        Response::Rules { rules } => Ok(rules),
        _ => Err("the daemon sent an unexpected answer".into()),
    }
}

fn describe(rule: &Rule) -> String {
    let w = &rule.when;
    let mut when = Vec::new();
    if let Some(h) = &w.host {
        when.push(format!("site {h}"));
    }
    if !w.extensions.is_empty() {
        when.push(format!("type {}", w.extensions.join(",")));
    }
    if let Some(c) = &w.url_contains {
        when.push(format!("address has {c:?}"));
    }
    if let Some(r) = &w.referrer_host {
        when.push(format!("from page on {r}"));
    }
    let t = &rule.then;
    let mut then = Vec::new();
    if t.block {
        then.push("BLOCK".to_string());
    }
    if let Some(f) = &t.folder {
        then.push(format!("folder {f}"));
    }
    if let Some(q) = t.queue {
        then.push(format!("queue {q}"));
    }
    match t.sort {
        Some(true) => then.push("sort by category".into()),
        Some(false) => then.push("do not sort".into()),
        None => {}
    }
    format!(
        "{:>3}  {}  {}\n       when {}\n       then {}",
        rule.id,
        if rule.enabled { "on " } else { "off" },
        rule.name,
        when.join(" and "),
        then.join(", ")
    )
}

fn split(text: Option<String>) -> Vec<String> {
    text.map(|t| {
        t.split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

async fn switch(client: &Client, id: i64, on: bool) -> Result<(), String> {
    let mut rules = list(client).await?;
    let Some(rule) = rules.iter_mut().find(|r| r.id == id) else {
        return Err(format!("no rule with number {id}"));
    };
    rule.enabled = on;
    call(client, Call::SetRules { rules }).await?;
    println!("Rule {id} is {}.", if on { "on" } else { "off" });
    Ok(())
}

async fn run_inner(command: RulesCommand, info: Option<PathBuf>) -> Result<(), String> {
    let client = client(info)?;
    match command {
        RulesCommand::List => {
            let rules = list(&client).await?;
            if rules.is_empty() {
                println!(
                    "No rules yet. Add one with: mohona rules add NAME --ext zip --folder Archives"
                );
            }
            for rule in &rules {
                println!("{}", describe(rule));
            }
        }
        RulesCommand::Add {
            name,
            host,
            ext,
            contains,
            referrer_host,
            folder,
            queue,
            sort,
            block,
        } => {
            let mut rules = list(&client).await?;
            rules.push(Rule {
                id: 0,
                name,
                enabled: true,
                when: When {
                    host,
                    extensions: split(ext),
                    url_contains: contains,
                    referrer_host,
                },
                then: Then {
                    folder,
                    queue,
                    sort: sort.then_some(true),
                    block,
                },
            });
            call(&client, Call::SetRules { rules }).await?;
            let saved = list(&client).await?;
            if let Some(last) = saved.last() {
                println!("Added rule {}.", last.id);
            }
        }
        RulesCommand::Remove { id } => {
            let mut rules = list(&client).await?;
            let before = rules.len();
            rules.retain(|r| r.id != id);
            if rules.len() == before {
                return Err(format!("no rule with number {id}"));
            }
            call(&client, Call::SetRules { rules }).await?;
            println!("Removed rule {id}.");
        }
        RulesCommand::Enable { id } => switch(&client, id, true).await?,
        RulesCommand::Disable { id } => switch(&client, id, false).await?,
        RulesCommand::Test { url, referrer } => {
            match call(&client, Call::TestRules { url, referrer }).await? {
                Response::RuleMatch { rule: Some(rule) } => println!("{}", describe(&rule)),
                _ => println!("No rule applies."),
            }
        }
    }
    Ok(())
}

pub async fn run(command: RulesCommand, info: Option<PathBuf>) -> ExitCode {
    match run_inner(command, info).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
