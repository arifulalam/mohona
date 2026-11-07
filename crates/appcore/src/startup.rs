//! Starting Mohona when the person signs in to Windows.
//!
//! Done with the per-user "Run" key of the registry (no administrator needed,
//! and the person can also see and switch it off in Windows' own Startup apps).
//! The program is started with `--minimized`, so it goes to the tray instead of
//! putting a window in front of whatever the person is about to do.

use std::path::Path;

use crate::AppError;

/// The name of the entry in the Run key.
pub const ENTRY: &str = "Mohona";
/// The argument the program is started with at sign-in.
pub const MINIMIZED: &str = "--minimized";
#[cfg_attr(not(windows), allow(dead_code))]
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// The command line stored in the registry for a program path.
pub fn run_line(exe: &Path) -> String {
    format!("\"{}\" {MINIMIZED}", exe.display())
}

/// Whether the command line the registry holds is for this program
/// (so a path that moved, after an update or a move of the folder, counts as off).
pub fn is_ours(stored: &str, exe: &Path) -> bool {
    stored.trim() == run_line(exe)
}

/// Takes the data of the value out of the output of `reg query`.
#[cfg_attr(not(windows), allow(dead_code))]
fn value_of(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let line = line.trim();
        let rest = line.strip_prefix(ENTRY)?.trim_start();
        let rest = rest.strip_prefix("REG_SZ")?;
        Some(rest.trim().to_string())
    })
}

/// Is Mohona set to start with Windows?
pub fn enabled(exe: &Path) -> bool {
    stored().is_some_and(|s| is_ours(&s, exe))
}

#[cfg(windows)]
fn reg() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new("reg");
    c.creation_flags(0x0800_0000);
    c
}

#[cfg(windows)]
fn stored() -> Option<String> {
    let out = reg().args(["query", KEY, "/v", ENTRY]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    value_of(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(windows))]
fn stored() -> Option<String> {
    None
}

/// Turns starting with Windows on or off for this program.
pub fn set_enabled(exe: &Path, on: bool) -> Result<(), AppError> {
    set_enabled_inner(exe, on)
}

#[cfg(windows)]
fn set_enabled_inner(exe: &Path, on: bool) -> Result<(), AppError> {
    let out = if on {
        reg()
            .args(["add", KEY, "/v", ENTRY, "/t", "REG_SZ", "/d"])
            .arg(run_line(exe))
            .arg("/f")
            .output()
    } else {
        if stored().is_none() {
            return Ok(());
        }
        reg().args(["delete", KEY, "/v", ENTRY, "/f"]).output()
    }
    .map_err(|e| AppError::Engine(format!("could not change the startup setting: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(AppError::Engine(format!(
            "could not change the startup setting: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

#[cfg(not(windows))]
fn set_enabled_inner(_exe: &Path, _on: bool) -> Result<(), AppError> {
    Err(AppError::Engine(
        "starting with the computer is set up for Windows only".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_line_quotes_the_path_and_asks_for_the_tray() {
        let exe = Path::new(r"C:\Program Files\Mohona\mohona-app.exe");
        let line = run_line(exe);
        assert_eq!(
            line,
            r#""C:\Program Files\Mohona\mohona-app.exe" --minimized"#
        );
        assert!(is_ours(&format!("  {line} "), exe));
        assert!(!is_ours(r#""D:\Old\mohona-app.exe" --minimized"#, exe));
    }

    #[test]
    fn the_value_is_read_from_the_output_of_reg() {
        let out = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\r\n    Mohona    REG_SZ    \"C:\\M\\m.exe\" --minimized\r\n\r\n";
        assert_eq!(
            value_of(out).as_deref(),
            Some("\"C:\\M\\m.exe\" --minimized")
        );
        assert_eq!(value_of("    Other    REG_SZ    x"), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn it_says_plainly_that_this_is_for_windows() {
        assert!(set_enabled(Path::new("/x"), true).is_err());
        assert!(!enabled(Path::new("/x")));
    }
}
