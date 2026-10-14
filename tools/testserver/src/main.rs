use std::net::SocketAddr;

use clap::Parser;
use mohona_testserver::{bind, serve, Config, DEFAULT_SIZE, USAGE};

/// Local test server for Mohona: serves generated files and misbehaves on purpose.
#[derive(Parser)]
#[command(name = "mohona-testserver", version)]
struct Args {
    /// Address to listen on
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,

    /// Default size of /file in bytes (default: 1 GiB)
    #[arg(long, default_value_t = DEFAULT_SIZE)]
    size: u64,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let listener = bind(args.listen).await?;
    println!("Listening on http://{}\n", listener.local_addr()?);
    println!("{USAGE}");
    serve(
        listener,
        Config {
            default_size: args.size,
        },
    )
    .await
}
