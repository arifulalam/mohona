//! `mohona remote ...`: the web companion for a phone or another computer on the same network.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use mohona_daemon::{Call, Client, Response};

const DEFAULT_PORT: u16 = 6792;

#[derive(Subcommand)]
pub enum RemoteCommand {
    /// Show whether the companion is on and where to open it
    Status,
    /// Switch it on (takes effect when the daemon restarts)
    Enable {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Switch it off (takes effect when the daemon restarts)
    Disable,
    /// Make a login token for the companion, shown once
    Token {
        /// A name to recognise it by (a phone, a laptop)
        #[arg(long, default_value = "remote")]
        name: String,
        /// The token may only look, not add or change downloads
        #[arg(long)]
        read_only: bool,
    },
}

async fn call(client: &Client, call: Call) -> Result<Response, String> {
    match client.call(call).await.map_err(|e| e.to_string())? {
        Response::Error { message } => Err(message),
        other => Ok(other),
    }
}

/// This computer's address on the local network (no packet is sent).
fn lan_address() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

async fn status(client: &Client) -> Result<(), String> {
    let Response::Remote { port, listening } = call(client, Call::GetRemote).await? else {
        return Err("the daemon sent an unexpected answer".into());
    };
    match (port, listening) {
        (0, _) => println!("The remote companion is off. Switch it on with: mohona remote enable"),
        (p, 0) => println!("The remote companion is set to port {p} but is not running yet (restart the daemon, or the port is taken)."),
        (_, l) => {
            let host = lan_address().map_or_else(|| "THIS-COMPUTER".to_string(), |a| a.to_string());
            println!("The remote companion is on: http://{host}:{l}/");
            println!("It uses plain HTTP: use it only on a network you trust. Login: mohona remote token");
        }
    }
    Ok(())
}

async fn run_inner(command: RemoteCommand, info: Option<PathBuf>) -> Result<(), String> {
    let path = info.unwrap_or_else(mohona_daemon::default_info_path);
    let client = Client::from_info_file(&path)
        .map_err(|_| "the daemon is not running. Start it with: mohona daemon".to_string())?;
    match command {
        RemoteCommand::Status => status(&client).await,
        RemoteCommand::Enable { port } => {
            if port == 0 {
                return Err("the port must be 1 to 65535".into());
            }
            call(&client, Call::SetRemote { port }).await?;
            println!("The remote companion will listen on port {port} after the daemon restarts (close the daemon, or the Mohona app, and start it again).");
            println!("Windows Firewall will ask once; allow it for private networks only.");
            Ok(())
        }
        RemoteCommand::Disable => {
            call(&client, Call::SetRemote { port: 0 }).await?;
            println!("The remote companion will stay off after the daemon restarts.");
            Ok(())
        }
        RemoteCommand::Token { name, read_only } => {
            let scope = if read_only { "read" } else { "write" }.to_string();
            match call(&client, Call::MakeToken { name, scope }).await? {
                Response::Token { token, scope, .. } => {
                    println!("Token ({scope}): {token}");
                    println!("Type it into the companion's page. It is shown only now; it can be revoked with DELETE /v1/tokens/ID.");
                    Ok(())
                }
                _ => Err("the daemon sent an unexpected answer".into()),
            }
        }
    }
}

pub async fn run(command: RemoteCommand, info: Option<PathBuf>) -> ExitCode {
    match run_inner(command, info).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
