use std::io::{stdin, stdout};
use std::path::PathBuf;
use std::process::ExitCode;

use mohona_daemon::{default_db_path, default_info_path};
use mohona_host::register::{self, Browser, Places, BROWSERS};
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

const USAGE: &str = "Usage:
  mohona-host install   [--browser chrome|chromium|edge|firefox|all] [--extension-id ID]
  mohona-host uninstall [--browser ...]
  mohona-host status
(Started by a browser, it speaks the native messaging protocol instead.)";

fn browsers(args: &[String]) -> Result<Vec<Browser>, String> {
    match option(args, "--browser").and_then(|p| p.to_str().map(str::to_string)) {
        None => Ok(BROWSERS.to_vec()),
        Some(name) if name == "all" => Ok(BROWSERS.to_vec()),
        Some(name) => Browser::parse(&name).map(|b| vec![b]).ok_or_else(|| {
            format!("unknown browser \"{name}\": use chrome, chromium, edge, firefox or all")
        }),
    }
}

/// `install`, `uninstall` and `status`: run by the person, not by a browser.
fn manage(command: &str, args: &[String]) -> Result<(), String> {
    let places = Places::current();
    let browsers = browsers(args)?;
    match command {
        "install" => {
            let host = std::env::current_exe().map_err(|e| e.to_string())?;
            let chromium_id = option(args, "--extension-id")
                .and_then(|p| p.to_str().map(str::to_string))
                .or_else(|| std::env::var("MOHONA_EXTENSION_ID").ok());
            if let Some(id) = &chromium_id {
                if !register::valid_chromium_id(id) {
                    return Err(format!(
                        "\"{id}\" is not an extension id (32 letters, a to p; see chrome://extensions)"
                    ));
                }
            }
            for browser in browsers {
                let allowed = if browser == Browser::Firefox {
                    register::FIREFOX_ID.to_string()
                } else if let Some(id) = &chromium_id {
                    id.clone()
                } else {
                    println!("{}: skipped (give --extension-id)", browser.name());
                    continue;
                };
                let done = register::install(&places, browser, &host, &[allowed])
                    .map_err(|e| format!("{}: {e}", browser.name()))?;
                println!(
                    "{}: registered ({})",
                    browser.name(),
                    done.manifest.display()
                );
            }
            Ok(())
        }
        "uninstall" => {
            for browser in browsers {
                let had = register::uninstall(&places, browser)
                    .map_err(|e| format!("{}: {e}", browser.name()))?;
                println!(
                    "{}: {}",
                    browser.name(),
                    if had { "removed" } else { "was not registered" }
                );
            }
            Ok(())
        }
        _ => {
            for browser in BROWSERS {
                match register::status(&places, browser) {
                    Some((program, allowed)) => println!(
                        "{}: registered for {} -> {}{}",
                        browser.name(),
                        allowed.join(", "),
                        program.display(),
                        if program.is_file() {
                            ""
                        } else {
                            "  (PROGRAM NOT FOUND)"
                        }
                    ),
                    None => println!("{}: not registered", browser.name()),
                }
            }
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(command) = args.first().map(String::as_str) {
        if matches!(command, "install" | "uninstall" | "status") {
            return match manage(command, &args[1..]) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("error: {message}");
                    ExitCode::FAILURE
                }
            };
        }
        if matches!(command, "help" | "--help" | "-h") {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
    }
    let info = option(&args, "--info")
        .or_else(|| std::env::var_os("MOHONA_INFO").map(PathBuf::from))
        .unwrap_or_else(default_info_path);
    let database = option(&args, "--db")
        .or_else(|| std::env::var_os("MOHONA_DB").map(PathBuf::from))
        .unwrap_or_else(default_db_path);
    let host = Host::new(info, Some(database), engine_program(&args));
    match serve(&host, &mut stdin().lock(), &mut stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Standard output belongs to the conversation; say it on stderr.
            eprintln!("mohona-host: {e}");
            ExitCode::FAILURE
        }
    }
}
