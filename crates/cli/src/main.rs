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

mod queue;

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
    /// Keep a list of downloads and run it with limits on how many go at once
    Queue {
        #[command(subcommand)]
        command: queue::QueueCommand,
    },
    /// Download a URL
    Get {
        /// The address to download (http or https)
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
    },
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
        }) => match get(url, output, usize::from(connections), verbose, max_speed).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
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
) -> Result<(), String> {
    let client = HttpClient::new(&ClientConfig::default()).map_err(|error| describe(&error))?;
    let mut request = DownloadRequest::new(url, output);
    request.connections = connections;
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
    Ok(())
}

/// An error and everything that caused it, joined with colons.
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
