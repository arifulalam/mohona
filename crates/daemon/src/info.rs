use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// How clients find the daemon: written when it starts, removed when it
/// stops. It holds the secret token, so it is readable only by its owner
/// (on Windows it lives in the person's own profile folder).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub version: String,
}

/// Where the daemon writes its info file unless told otherwise: in the
/// person's own application-data folder, so other accounts cannot read it.
pub fn default_info_path() -> std::path::PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
            })
    };
    base.unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Mohona")
        .join("daemon.json")
}

/// The database shared by the engine and everything that starts it for the
/// browser: next to the info file.
pub fn default_db_path() -> std::path::PathBuf {
    default_info_path().with_file_name("mohona.db")
}

/// Where downloads go when nobody names a folder: the person's Downloads.
pub fn default_download_dir() -> std::path::PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from)
        .join("Downloads")
}

impl DaemonInfo {
    /// A new random token, 32 bytes in hex.
    pub fn new_token() -> String {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the system has a source of randomness");
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn read(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read(path)?;
        serde_json::from_slice(&text).map_err(std::io::Error::other)
    }

    /// Writes the file atomically (a temporary file, then a rename).
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = std::path::PathBuf::from(temporary);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    }
}
