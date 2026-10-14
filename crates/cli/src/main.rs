use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mohona",
    version,
    about = "Mohona: an estuary for your downloads"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Download a URL (arrives in Phase 1)
    Get {
        /// The address to download
        url: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Get { url }) => {
            eprintln!(
                "mohona {}: downloading is not implemented yet (Phase 1). URL: {url}",
                mohona_core::VERSION
            );
            std::process::exit(2);
        }
        None => println!(
            "mohona {} - an estuary for your downloads. Run with --help for commands.",
            mohona_core::VERSION
        ),
    }
}
