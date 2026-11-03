use std::error::Error;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::{Parser, Subcommand};
use mohona_core::{
    download, partial_progress, CancelToken, DownloadRequest, Error as CoreError, Event,
    DEFAULT_CONNECTIONS,
};
use mohona_net::{ClientConfig, HttpClient};
use tokio::sync::mpsc;

mod daemon;
mod history;
mod profiles;
mod queue;
mod remote;
mod resolve;
mod rules;
mod stream;
mod tools;

#[derive(Parser)]
#[command(
    name = "mohona",
    version,
    about = "Mohona: an estuary for your downloads"
)]
struct Cli {
    /// The database file that keeps the download list
    #[arg(long, global = true, default_value = "mohona.db")]
    db: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run Mohona in the background so the browser extension can add downloads
    Daemon {
        /// Where to write the connection details (default: your application-data folder)
        #[arg(long)]
        info: Option<PathBuf>,
        /// Folder for downloads that do not name one (default: Downloads)
        #[arg(long)]
        folder: Option<PathBuf>,
    },
    /// Show where the local API is and the master token (the daemon must be running)
    Api {
        /// The daemon's connection file (default: your application-data folder)
        #[arg(long)]
        info: Option<PathBuf>,
    },
    /// Rules: when a download matches this, do that (the daemon must be running)
    Rules {
        #[command(subcommand)]
        command: rules::RulesCommand,
        /// The daemon's connection file (default: your application-data folder)
        #[arg(long)]
        info: Option<PathBuf>,
    },
    /// Show the direct download address of a cloud share link (Google Drive, Dropbox, OneDrive, GitHub ...)
    Resolve {
        /// The share link
        url: String,
    },
    /// The web companion for a phone or another computer on your network (the daemon must be running)
    Remote {
        #[command(subcommand)]
        command: remote::RemoteCommand,
        /// The daemon's connection file (default: your application-data folder)
        #[arg(long)]
        info: Option<PathBuf>,
    },
    /// Per-site profiles: connections, speed and headers for one site (the daemon must be running)
    Profiles {
        #[command(subcommand)]
        command: profiles::ProfilesCommand,
        /// The daemon's connection file (default: your application-data folder)
        #[arg(long)]
        info: Option<PathBuf>,
    },
    /// Keep a list of downloads and run it with limits on how many go at once
    Queue {
        #[command(subcommand)]
        command: queue::QueueCommand,
    },
    /// Search the list by name, address or category, newest first
    History {
        /// Words to find (each must match the start of a word)
        text: Vec<String>,
        /// Only this state, like completed or failed
        #[arg(long)]
        status: Option<String>,
        /// Only downloads added on or after this day, like 2026-10-01
        #[arg(long, value_parser = history::parse_date)]
        since: Option<i64>,
        /// Only downloads added on or before this day
        #[arg(long, value_parser = history::parse_date)]
        until: Option<i64>,
        /// Only this category
        #[arg(long)]
        category: Option<String>,
        /// How many to show
        #[arg(short, long, default_value_t = 50)]
        limit: u32,
    },
    /// Show the network addresses Mohona watches for changes
    Network,
    /// Show what happened to a download
    Log {
        /// The number shown by `queue list`
        id: i64,
    },
    /// Open the finished start of an unfinished download (or any file) in your player
    Preview {
        /// The address of the unfinished download, or a file
        target: String,

        /// Folder the download is saved in (as given to `get`)
        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        /// Only make the preview file and print where it is
        #[arg(long)]
        no_open: bool,
    },
    /// Convert a file with ffmpeg: get the sound, make it smaller, make it play everywhere
    Convert {
        /// The file to convert
        file: Option<PathBuf>,

        /// mp3, m4a, mp4 or mp4-small
        #[arg(short, long, default_value = "mp4")]
        preset: String,

        /// Show the presets and exit
        #[arg(long)]
        list: bool,

        /// Folder for the result (default: next to the file)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Where ffmpeg is (the program, or the folder holding it)
        #[arg(long)]
        ffmpeg: Option<PathBuf>,
    },
    /// Download an HLS (.m3u8) or DASH (.mpd) stream
    Stream {
        /// The address of the playlist or manifest
        url: String,

        /// Folder to save the file in
        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        /// Name for the file, without the extension (default: from the address)
        #[arg(short, long)]
        name: Option<String>,

        /// best, worst, audio, or a height like 720 (the best one not taller than that)
        #[arg(short, long, default_value = "best")]
        quality: String,

        /// Audio language or name to download, like en or Bangla
        #[arg(long)]
        audio: Option<String>,

        /// Only show the qualities and languages the stream offers
        #[arg(long)]
        list: bool,

        /// Keep video and audio as two files instead of joining them with ffmpeg
        #[arg(long)]
        no_join: bool,

        /// For a live stream: stop after this much video, like 90s, 30m or 1h30m (default: until it ends or Ctrl-C)
        #[arg(long, value_parser = stream::parse_duration)]
        record_for: Option<f64>,

        /// Also download subtitles: a language like en, several like en,bn, or all (saved as NAME.LANG.srt)
        #[arg(long)]
        subs: Option<String>,

        /// Put the downloaded subtitles into the video file as tracks (needs ffmpeg; the .srt files stay)
        #[arg(long, requires = "subs")]
        embed_subs: bool,

        /// File type to join into: mp4 (default) or mkv
        #[arg(long, default_value = "mp4")]
        container: String,

        /// Where ffmpeg is (the program, or the folder holding it); default: MOHONA_FFMPEG, next to mohona, or the PATH
        #[arg(long)]
        ffmpeg: Option<PathBuf>,

        /// Segments to fetch at once (1 to 32)
        #[arg(short = 'c', long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=32))]
        connections: u16,

        /// Speed limit, like 500K or 1.5M (bytes per second)
        #[arg(short = 'l', long, value_parser = parse_rate)]
        max_speed: Option<u64>,

        #[command(flatten)]
        request: RequestArgs,
    },
    /// Download a URL
    Get {
        /// The address to download (http, https, webdav, ftp, ftps, ftpes or sftp)
        url: String,

        /// Folder to save the file in
        #[arg(short, long, default_value = ".")]
        output: PathBuf,

        /// Print what the download is deciding (connections added or dropped, server pushback)
        #[arg(short, long)]
        verbose: bool,

        /// Most connections to use at once (1 to 32)
        #[arg(short = 'c', long, default_value_t = DEFAULT_CONNECTIONS as u16, value_parser = clap::value_parser!(u16).range(1..=32))]
        connections: u16,

        /// Speed limit, like 500K or 1.5M (bytes per second)
        #[arg(short = 'l', long, value_parser = parse_rate)]
        max_speed: Option<u64>,

        #[command(flatten)]
        request: RequestArgs,
    },
}

/// `user:password`, or just `user` with the password from `fallback`.
fn parse_login(text: &str, fallback: Option<String>) -> Result<mohona_net::Credentials, String> {
    if let Some(login) = mohona_net::Credentials::parse(text) {
        return Ok(login);
    }
    match (text.is_empty() || text.contains(':'), fallback) {
        (false, Some(password)) => Ok(mohona_net::Credentials {
            username: text.to_string(),
            password,
        }),
        _ => Err("give --user as name:password, or as a name with the password in the MOHONA_PASSWORD environment variable".into()),
    }
}

/// What to send with a download besides the address.
#[derive(clap::Args, Clone, Default)]
pub(crate) struct RequestArgs {
    /// Send a header, like -H "Accept-Language: bn" (can be repeated)
    #[arg(short = 'H', long = "header")]
    headers: Vec<String>,
    /// Send cookies, like --cookie "session=abc; theme=dark"
    #[arg(long)]
    cookie: Option<String>,
    /// The page the link was on (the Referer header)
    #[arg(long)]
    referer: Option<String>,
    /// Pretend to be this browser (the User-Agent header)
    #[arg(long)]
    user_agent: Option<String>,
    /// Log in with user:password (Basic or Digest, when the server asks).
    /// Without a password, MOHONA_PASSWORD is used. A user:password@ in the
    /// address works too.
    #[arg(short = 'u', long)]
    user: Option<String>,
    /// Go through a proxy: http://host:port, socks5://host:port or
    /// socks5h://host:port (put user:password@ in it if it needs a login)
    #[arg(long, value_name = "URL", conflicts_with = "no_proxy")]
    proxy: Option<String>,
    /// Do not use any proxy, not even the system one
    #[arg(long)]
    no_proxy: bool,
    /// Also trust the certificate authority in this PEM file (for a server
    /// with its own certificate)
    #[arg(long, value_name = "FILE")]
    ca_file: Option<std::path::PathBuf>,
    /// Accept any certificate (or SSH host key), even a wrong one. Unsafe: use
    /// only for a server you know
    #[arg(long)]
    insecure: bool,
    /// SFTP: log in with this private key file (a passphrase is read from
    /// MOHONA_KEY_PASSPHRASE and never saved)
    #[arg(long, value_name = "FILE")]
    key: Option<std::path::PathBuf>,
    /// SFTP: trust only the server with this key fingerprint (SHA256:...)
    #[arg(long, value_name = "FINGERPRINT")]
    host_key: Option<String>,
    /// SFTP: the file of known servers (default: ~/.ssh/known_hosts)
    #[arg(long, value_name = "FILE")]
    known_hosts: Option<std::path::PathBuf>,
    /// SFTP: remember a server seen for the first time (in known_hosts).
    /// A server whose key changed is never accepted this way
    #[arg(long)]
    trust_new_host: bool,
    /// What the finished file must hash to, like sha256:ABC... (or just the
    /// hash: md5, sha1, sha256 and sha512 are told apart by length). A
    /// file that does not match fails, and is kept
    #[arg(long, value_name = "HASH")]
    checksum: Option<String>,
    /// Look for a checksum file next to the download (address.sha256, .sha512,
    /// .sha1 or .md5) and check against it. Without this, only hashes the
    /// server announces itself are checked
    #[arg(long)]
    find_checksum: bool,
    /// Another address of the same file (can be repeated). The download uses
    /// all of them together and moves on when one fails. They must hold the
    /// same file; the size is checked, so use --checksum to be sure
    #[arg(long = "mirror", value_name = "URL")]
    mirrors: Vec<String>,
}

impl RequestArgs {
    pub(crate) fn context(&self) -> Result<mohona_net::RequestContext, String> {
        let mut context = mohona_net::RequestContext {
            // MOHONA_COOKIE keeps a session cookie out of the process list
            // (the browser host uses it).
            cookies: self.cookie.clone().or_else(|| {
                std::env::var("MOHONA_COOKIE")
                    .ok()
                    .filter(|c| !c.is_empty())
            }),
            referrer: self.referer.clone(),
            user_agent: self.user_agent.clone(),
            ..Default::default()
        };
        for line in &self.headers {
            context.add_header_line(line).map_err(|e| e.to_string())?;
        }
        if let Some(user) = &self.user {
            context.credentials = Some(parse_login(user, std::env::var("MOHONA_PASSWORD").ok())?);
        }
        if let Some(proxy) = &self.proxy {
            context.proxy = Some(mohona_net::ProxyChoice::parse(proxy).map_err(|e| e.to_string())?);
        } else if self.no_proxy {
            context.proxy = Some(mohona_net::ProxyChoice::Direct);
        }
        if let Some(text) = &self.checksum {
            context.checksum = Some(mohona_net::Checksum::parse(text).map_err(|e| e.to_string())?);
        }
        context.find_checksum = self.find_checksum;
        context.mirrors.clone_from(&self.mirrors);
        if self.key.is_some()
            || self.host_key.is_some()
            || self.known_hosts.is_some()
            || self.trust_new_host
        {
            let full = |path: &std::path::PathBuf| -> Result<String, String> {
                Ok(std::path::absolute(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?
                    .to_string_lossy()
                    .into_owned())
            };
            context.ssh = Some(mohona_net::SshOptions {
                key_file: self.key.as_ref().map(full).transpose()?,
                host_key: self.host_key.clone(),
                known_hosts: self.known_hosts.as_ref().map(full).transpose()?,
                trust_new_host: self.trust_new_host,
            });
        }
        if self.insecure || self.ca_file.is_some() {
            let ca_file = match &self.ca_file {
                // Saved with the download, so keep the full path.
                Some(path) => Some(
                    std::path::absolute(path)
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .to_string_lossy()
                        .into_owned(),
                ),
                None => None,
            };
            context.tls = Some(mohona_net::TlsOptions {
                insecure: self.insecure,
                ca_file,
            });
        }
        Ok(context)
    }
}

/// Read a speed such as `500K`, `1.5M`, `2G` or a plain number of bytes.
/// `off` and `0` mean no limit (0).
pub(crate) fn parse_rate(text: &str) -> Result<u64, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("off") {
        return Ok(0);
    }
    let (number, multiplier) = match text.chars().last().map(|c| c.to_ascii_uppercase()) {
        Some('K') => (&text[..text.len() - 1], 1024.0),
        Some('M') => (&text[..text.len() - 1], 1024.0 * 1024.0),
        Some('G') => (&text[..text.len() - 1], 1024.0 * 1024.0 * 1024.0),
        _ => (text, 1.0),
    };
    let value: f64 = number
        .trim()
        .parse()
        .map_err(|_| format!("'{text}' is not a speed; try 500K, 1.5M or off"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("'{text}' is not a speed; try 500K, 1.5M or off"));
    }
    Ok((value * multiplier) as u64)
}

#[cfg(test)]
mod rate_tests {
    use super::parse_rate;

    #[test]
    fn speeds_are_read_with_suffixes() {
        assert_eq!(parse_rate("500"), Ok(500));
        assert_eq!(parse_rate("500K"), Ok(512_000));
        assert_eq!(parse_rate("1m"), Ok(1_048_576));
        assert_eq!(parse_rate("1.5M"), Ok(1_572_864));
        assert_eq!(parse_rate("off"), Ok(0));
        assert_eq!(parse_rate("0"), Ok(0));
        assert!(parse_rate("fast").is_err());
        assert!(parse_rate("-1M").is_err());
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Get {
            url,
            output,
            connections,
            verbose,
            max_speed,
            request,
        }) => match get(
            url,
            output,
            usize::from(connections),
            verbose,
            max_speed,
            request,
        )
        .await
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
        Some(Command::Preview {
            target,
            output,
            no_open,
        }) => finish(tools::preview(&target, &output, no_open)),
        Some(Command::Convert {
            file,
            preset,
            list,
            output,
            ffmpeg,
        }) => finish(if list {
            tools::list_presets();
            Ok(())
        } else {
            match file {
                Some(file) => tools::convert(&file, &preset, output.as_deref(), ffmpeg.as_deref()),
                None => Err("give the file to convert (or --list to see the presets)".to_string()),
            }
        }),
        Some(Command::Stream {
            url,
            output,
            name,
            quality,
            audio,
            list,
            no_join,
            record_for,
            subs,
            embed_subs,
            container,
            ffmpeg,
            connections,
            max_speed,
            request,
        }) => finish(
            stream::run(stream::StreamArgs {
                url,
                output,
                name,
                quality,
                audio,
                list,
                no_join,
                record_for,
                subs,
                embed_subs,
                container,
                ffmpeg,
                connections: usize::from(connections),
                max_speed,
                request,
            })
            .await,
        ),
        Some(Command::History {
            text,
            status,
            since,
            until,
            category,
            limit,
        }) => finish(open(&cli.db).and_then(|store| {
            history::history(
                &store,
                history::Filters {
                    text,
                    status,
                    since,
                    until,
                    category,
                    limit,
                },
            )
        })),
        Some(Command::Api { info }) => {
            let path = info.unwrap_or_else(mohona_daemon::default_info_path);
            match mohona_daemon::DaemonInfo::read(&path) {
                Ok(i) if i.api_port != 0 => {
                    println!("Address: http://127.0.0.1:{}/v1/", i.api_port);
                    println!("Token:   {}", i.token);
                    println!("Keep the token secret. Make smaller tokens with POST /v1/tokens.");
                    ExitCode::SUCCESS
                }
                Ok(_) => {
                    eprintln!("The local API is off (MOHONA_API_PORT=off).");
                    ExitCode::FAILURE
                }
                Err(_) => {
                    eprintln!("The daemon is not running. Start it with: mohona daemon");
                    ExitCode::FAILURE
                }
            }
        }
        Some(Command::Network) => {
            use mohona_manager::NetworkProbe;
            let state = mohona_manager::SystemNetwork.state();
            let show =
                |a: Option<std::net::IpAddr>| a.map_or("none".to_string(), |a| a.to_string());
            println!("IPv4 route uses: {}", show(state.v4));
            println!("IPv6 route uses: {}", show(state.v6));
            println!(
                "{}",
                if state.online() {
                    "Online."
                } else {
                    "Offline."
                }
            );
            ExitCode::SUCCESS
        }
        Some(Command::Log { id }) => {
            finish(open(&cli.db).and_then(|store| history::log(&store, id)))
        }
        Some(Command::Daemon { info, folder }) => match daemon::run(
            // Without `--db`, use the database the browser host also uses.
            if cli.db == std::path::Path::new("mohona.db") {
                mohona_daemon::default_db_path()
            } else {
                cli.db
            },
            info,
            folder,
        )
        .await
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
        Some(Command::Resolve { url }) => resolve::run(&url),
        Some(Command::Remote { command, info }) => remote::run(command, info).await,
        Some(Command::Profiles { command, info }) => profiles::run(command, info).await,
        Some(Command::Rules { command, info }) => rules::run(command, info).await,
        Some(Command::Queue { command }) => match queue::run(command, cli.db).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
        None => {
            println!(
                "mohona {} - an estuary for your downloads. Run with --help for commands.",
                mohona_core::VERSION
            );
            ExitCode::SUCCESS
        }
    }
}

async fn get(
    url: String,
    output: PathBuf,
    connections: usize,
    verbose: bool,
    max_speed: Option<u64>,
    args: RequestArgs,
) -> Result<(), String> {
    let client = HttpClient::new(&ClientConfig::default()).map_err(|error| describe(&error))?;
    // A password written in the address is taken out of it, so it is only
    // sent when the server asks for it.
    let (url, from_address) = mohona_net::split_userinfo(&url);
    let mut request = DownloadRequest::new(url, output);
    request.connections = connections;
    request.context = args.context()?;
    if request.context.credentials.is_none() {
        request.context.credentials = from_address;
    }
    if let Some(rate) = max_speed.filter(|rate| *rate > 0) {
        request.limits = vec![mohona_core::RateLimit::new(rate)];
    }
    // Ctrl-C stops the download cleanly and keeps what was fetched.
    let cancel = CancelToken::new();
    request.cancel = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancel.cancel();
        }
    });
    let (dest, address) = (request.dest_dir.clone(), request.url.clone());

    let (sender, mut receiver) = mpsc::channel(64);
    let worker = tokio::spawn(async move { download(&client, &request, Some(&sender)).await });

    let mut display = Display::new(verbose);
    while let Some(event) = receiver.recv().await {
        display.handle(&event);
    }

    let result = worker.await.map_err(|error| error.to_string())?;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            display.finish_line();
            let mut message = if matches!(error, CoreError::Cancelled) {
                "stopped".to_string()
            } else {
                describe(&error)
            };
            if let Some((saved, total)) = partial_progress(&dest, &address) {
                message.push_str(&format!(
                    "\nKept {} of {}. Run the same command again to resume.",
                    format_bytes(saved),
                    format_bytes(total)
                ));
            }
            return Err(message);
        }
    };
    println!(
        "Saved {} ({})",
        outcome.path.display(),
        format_bytes(outcome.bytes)
    );
    if let Some(verified) = &outcome.verified {
        let from = match &verified.source {
            mohona_core::ChecksumSource::Given => "the checksum you gave".to_string(),
            mohona_core::ChecksumSource::File(url) => url.clone(),
            mohona_core::ChecksumSource::Header => "the server's headers".to_string(),
        };
        println!(
            "Checked: {} matches ({from})",
            verified.checksum.algorithm.name()
        );
    }
    Ok(())
}

/// An error and everything that caused it, joined with colons.
fn open(db: &std::path::Path) -> Result<mohona_store::Store, String> {
    mohona_store::Store::open(db).map_err(|e| format!("cannot open {}: {e}", db.display()))
}

fn finish(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn describe(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        let part = inner.to_string();
        if !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = inner.source();
    }
    text
}

/// Draws the progress line on the terminal.
struct Display {
    started: Instant,
    interactive: bool,
    drew_line: bool,
    told_limited: bool,
    verbose: bool,
    /// Bytes that were already on disk when this run started.
    resumed: u64,
}

impl Display {
    fn new(verbose: bool) -> Self {
        Self {
            started: Instant::now(),
            interactive: std::io::stderr().is_terminal(),
            drew_line: false,
            told_limited: false,
            verbose,
            resumed: 0,
        }
    }

    fn handle(&mut self, event: &Event) {
        match event {
            Event::Started {
                filename,
                total,
                resumable,
                resumed,
                connections,
                ..
            } => {
                self.resumed = *resumed;
                self.started = Instant::now();
                let size = total.map_or("unknown size".to_string(), format_bytes);
                let resume = if *resumable { "" } else { ", cannot resume" };
                let lanes = if *connections > 1 {
                    format!(", up to {connections} connections")
                } else {
                    String::new()
                };
                if *resumed > 0 {
                    eprintln!(
                        "Resuming {filename} at {} ({size}{resume}{lanes})",
                        format_bytes(*resumed)
                    );
                } else {
                    eprintln!("Downloading {filename} ({size}{resume}{lanes})");
                }
            }
            Event::Progress { downloaded, total } => self.draw(*downloaded, *total),
            Event::Parts(_) => {}
            Event::Note(text) => {
                if self.verbose {
                    if self.drew_line {
                        eprint!("\r{:<79}\r", "");
                    }
                    eprintln!("  [{:>5.1}s] {text}", self.started.elapsed().as_secs_f64());
                }
            }
            Event::Limited { .. } => {
                // The count drops one step at a time; say it only once.
                if !self.told_limited {
                    self.told_limited = true;
                    if self.drew_line {
                        eprint!("\r{:<79}\r", "");
                    }
                    eprintln!("The server is limiting simultaneous connections; using fewer");
                }
            }
            Event::Finished { .. } => {
                if self.drew_line {
                    eprintln!();
                }
            }
        }
    }

    /// Leave the progress line, so the next message starts on a fresh line.
    fn finish_line(&mut self) {
        if self.drew_line {
            eprintln!();
            self.drew_line = false;
        }
    }

    fn draw(&mut self, downloaded: u64, total: Option<u64>) {
        if !self.interactive {
            return;
        }
        let elapsed = self.started.elapsed().as_secs_f64().max(0.001);
        // Only what this run fetched counts for the speed.
        let speed = downloaded.saturating_sub(self.resumed) as f64 / elapsed;
        let line = match total {
            Some(total) if total > 0 => {
                let fraction = (downloaded as f64 / total as f64).clamp(0.0, 1.0);
                let remaining = total.saturating_sub(downloaded);
                let eta = if speed > 0.0 {
                    format_duration((remaining as f64 / speed).round() as u64)
                } else {
                    "--:--".to_string()
                };
                format!(
                    "{} {:>3.0}%  {} / {}  {}/s  ETA {}",
                    bar(fraction),
                    fraction * 100.0,
                    format_bytes(downloaded),
                    format_bytes(total),
                    format_bytes(speed as u64),
                    eta
                )
            }
            _ => format!(
                "{}  {}/s",
                format_bytes(downloaded),
                format_bytes(speed as u64)
            ),
        };
        // `\r` returns to the start of the line; the spaces wipe leftovers.
        eprint!("\r{line:<79}");
        let _ = std::io::stderr().flush();
        self.drew_line = true;
    }
}

fn bar(fraction: f64) -> String {
    const WIDTH: usize = 24;
    let filled = (fraction * WIDTH as f64).round() as usize;
    format!("[{}{}]", "#".repeat(filled), "-".repeat(WIDTH - filled))
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_duration(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::{bar, format_bytes, format_duration};

    #[test]
    fn formats_sizes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1_572_864), "1.5 MiB");
        assert_eq!(format_bytes(1 << 30), "1.0 GiB");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(5), "0:05");
        assert_eq!(format_duration(65), "1:05");
        assert_eq!(format_duration(3_725), "1:02:05");
    }

    #[test]
    fn bar_fills_from_the_left() {
        assert_eq!(bar(0.0), format!("[{}]", "-".repeat(24)));
        assert_eq!(bar(1.0), format!("[{}]", "#".repeat(24)));
        assert_eq!(bar(0.5), format!("[{}{}]", "#".repeat(12), "-".repeat(12)));
    }
}
