//! `mohona grab URL`: list the files a web page and the pages it links to offer, and download
//! them if asked (the daemon must be running).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use mohona_daemon::{Call, Client, Response};

#[derive(Args)]
pub struct GrabArgs {
    /// The web page to start from
    pub url: String,
    /// How many links deep to go: 0 is this page only, 1 also the pages it links to, up to 5
    #[arg(long, default_value_t = 1)]
    pub depth: u32,
    /// Also follow links to other sites (by default only pages of the same site are followed)
    #[arg(long)]
    pub any_site: bool,
    /// Only these files: groups (images, video, audio, documents, archives, programs) and/or
    /// endings (pdf,zip), like `--types images,pdf`
    #[arg(long, value_delimiter = ',')]
    pub types: Vec<String>,
    /// Read at most this many pages (default 100, at most 500)
    #[arg(long)]
    pub max_pages: Option<u32>,
    /// Do not obey the site's robots.txt (be considerate: it is the site's request)
    #[arg(long)]
    pub ignore_robots: bool,
    /// Download everything found (without this, only the list is shown)
    #[arg(long)]
    pub download: bool,
    /// With --download: save in this folder (default: the download folder)
    #[arg(long, requires = "download")]
    pub folder: Option<String>,
    /// The daemon's connection file (default: your application-data folder)
    #[arg(long)]
    pub info: Option<PathBuf>,
}

async fn run_inner(args: GrabArgs) -> Result<(), String> {
    let path = args
        .info
        .clone()
        .unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    if args.depth > 5 {
        return Err("--depth can be at most 5".into());
    }
    let answer = client
        .call(Call::GrabStart {
            url: args.url.clone(),
            depth: args.depth,
            same_site: !args.any_site,
            types: args.types.clone(),
            max_pages: args.max_pages,
            robots: !args.ignore_robots,
        })
        .await
        .map_err(|e| e.to_string())?;
    if let Response::Error { message } = answer {
        return Err(message);
    }
    let stop = {
        let client = client.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = client.call(Call::GrabStop).await;
        })
    };
    let mut last = (0, 0);
    let result = loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        match client
            .call(Call::GrabStatus)
            .await
            .map_err(|e| e.to_string())?
        {
            Response::Grab {
                state,
                pages,
                found,
                error,
                files,
                skipped_by_robots,
                failed_pages,
                note,
                ..
            } => {
                if state == "running" {
                    if (pages, found) != last {
                        eprintln!("{pages} pages read, {found} files found...");
                        last = (pages, found);
                    }
                    continue;
                }
                if let Some(e) = error {
                    return Err(e);
                }
                break (files, pages, skipped_by_robots, failed_pages, note);
            }
            Response::Error { message } => return Err(message),
            _ => return Err("the daemon sent an unexpected answer".into()),
        }
    };
    stop.abort();
    let (files, pages, skipped, failed, note) = result;
    for f in &files {
        println!("{:<9} {}  {}", f.group, f.name, f.url);
    }
    eprintln!("{} files found on {pages} pages.", files.len());
    if skipped > 0 {
        eprintln!("{skipped} pages were left alone because the site's robots.txt asks for it (--ignore-robots overrides).");
    }
    if failed > 0 {
        eprintln!("{failed} pages could not be read.");
    }
    if let Some(note) = note {
        eprintln!("Note: {note}.");
    }
    if args.download {
        let mut added = 0;
        for f in &files {
            let call = Call::Add {
                url: f.url.clone(),
                dest_dir: args.folder.clone(),
                cookies: None,
                referrer: Some(f.page.clone()),
                user_agent: None,
                headers: Vec::new(),
                sort: false,
                duplicates: None,
                queue: None,
                files: None,
                video: false,
                choice: None,
            };
            match client.call(call).await.map_err(|e| e.to_string())? {
                Response::Added { .. } => added += 1,
                Response::Skipped { reason } => eprintln!("{}: not added: {reason}", f.name),
                Response::Error { message } => eprintln!("{}: {message}", f.name),
                _ => {}
            }
        }
        eprintln!("{added} downloads added.");
    }
    Ok(())
}

pub async fn run(args: GrabArgs) -> ExitCode {
    match run_inner(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
