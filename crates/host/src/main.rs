use std::io::{stdin, stdout};
use std::path::PathBuf;
use std::process::ExitCode;

use mohona_daemon::default_info_path;
use mohona_host::{serve, Host};

/// The value after `flag` (the browser's own arguments, such as the
/// extension's origin, are ignored).
fn option(args: &[String], flag: &str) -> Option<PathBuf> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(PathBuf::from)
}

fn engine_program(args: &[String]) -> Option<PathBuf> {
    if let Some(path) = option(args, "--engine") {
        return Some(path);
    }
    let name = if cfg!(windows) {
        "mohona.exe"
    } else {
        "mohona"
    };
    let beside = std::env::current_exe().ok()?.parent()?.join(name);
    beside.is_file().then_some(beside)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let info = option(&args, "--info")
        .or_else(|| std::env::var_os("MOHONA_INFO").map(PathBuf::from))
        .unwrap_or_else(default_info_path);
    let database =
        option(&args, "--db").or_else(|| std::env::var_os("MOHONA_DB").map(PathBuf::from));
    let host = Host::new(info, database, engine_program(&args));
    match serve(&host, &mut stdin().lock(), &mut stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Standard output belongs to the conversation; say it on stderr.
            eprintln!("mohona-host: {e}");
            ExitCode::FAILURE
        }
    }
}
