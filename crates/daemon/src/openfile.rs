//! Opening a finished file, or showing it in its folder, with the system's own tools.
//!
//! The path always comes from the engine's own list, never from a web page or a browser message.

use std::path::Path;

/// Files that run code when opened (programs, scripts, installers, shortcuts, macros).
pub fn is_risky(path: &std::path::Path) -> bool {
    const RISKY: &[&str] = &[
        "exe", "com", "scr", "msi", "msp", "msix", "appx", "bat", "cmd", "ps1", "psm1", "vbs",
        "vbe", "js", "jse", "wsf", "wsh", "hta", "lnk", "url", "reg", "dll", "cpl", "msc", "pif",
        "jar", "sh", "bash", "command", "app", "apk", "dmg", "pkg", "deb", "rpm", "appimage",
        "docm", "xlsm", "pptm", "gadget", "inf", "scf", "chm",
    ];
    // The extension is whatever follows the last dot, and a trailing dot or
    // space (which Windows ignores) does not hide it.
    let name = path
        .file_name()
        .map(|n| {
            n.to_string_lossy()
                .trim_end_matches(['.', ' '])
                .to_lowercase()
        })
        .unwrap_or_default();
    match name.rsplit_once('.') {
        Some((_, ext)) => RISKY.contains(&ext),
        None => false,
    }
}

fn spawn(mut command: std::process::Command) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // no console window
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    // Reap it in the background so it does not stay a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Opens a file with the program the system has chosen for its type.
pub fn open(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let command = {
        let mut c = std::process::Command::new("explorer.exe");
        c.arg(path);
        c
    };
    #[cfg(target_os = "macos")]
    let command = {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    spawn(command)
}

/// Shows a file in its folder, selected where the system can do that.
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new("explorer.exe");
        c.raw_arg(format!("/select,\"{}\"", path.display()));
        c
    };
    #[cfg(target_os = "macos")]
    let command = {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    spawn(command)
}
