//! Telling browsers where the host is.
//!
//! A browser only starts a native messaging host that is *registered*: a small
//! JSON manifest names the program and the extension allowed to use it, and
//! the browser finds the manifest in a fixed place (a registry key on Windows,
//! a folder on macOS and Linux). Everything here is per user; nothing needs
//! administrator rights.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The name browsers know the host by.
pub const HOST_NAME: &str = "org.mohona.host";

/// The id of the extension built from the `extension` folder: it pins its
/// key (`extension/identity.json`), so the id is the same everywhere. A test
/// in the extension checks that the two agree.
pub const CHROMIUM_ID: &str = "iflnmaneimjgoibmpcmlkllhjccebbhj";

/// The Firefox add-on id the extension will carry (Firefox identifies
/// extensions by this id, Chromium browsers by a key-derived one).
pub const FIREFOX_ID: &str = "mohona@mohona.local";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Chromium,
    Edge,
    Firefox,
}

pub const BROWSERS: [Browser; 4] = [
    Browser::Chrome,
    Browser::Chromium,
    Browser::Edge,
    Browser::Firefox,
];

impl Browser {
    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "chrome" => Some(Self::Chrome),
            "chromium" => Some(Self::Chromium),
            "edge" => Some(Self::Edge),
            "firefox" => Some(Self::Firefox),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Chromium => "chromium",
            Self::Edge => "edge",
            Self::Firefox => "firefox",
        }
    }

    fn is_firefox(self) -> bool {
        self == Self::Firefox
    }

    /// The registry key (below `HKEY_CURRENT_USER`) on Windows.
    fn registry_key(self) -> String {
        let vendor = match self {
            Self::Chrome => r"Google\Chrome",
            Self::Chromium => "Chromium",
            Self::Edge => r"Microsoft\Edge",
            Self::Firefox => "Mozilla",
        };
        format!(r"Software\{vendor}\NativeMessagingHosts\{HOST_NAME}")
    }

    /// The folder the manifest goes in on macOS and Linux.
    fn manifest_folder(self, home: &Path, mac: bool) -> PathBuf {
        if mac {
            let support = home.join("Library/Application Support");
            match self {
                Self::Chrome => support.join("Google/Chrome/NativeMessagingHosts"),
                Self::Chromium => support.join("Chromium/NativeMessagingHosts"),
                Self::Edge => support.join("Microsoft Edge/NativeMessagingHosts"),
                Self::Firefox => support.join("Mozilla/NativeMessagingHosts"),
            }
        } else {
            match self {
                Self::Chrome => home.join(".config/google-chrome/NativeMessagingHosts"),
                Self::Chromium => home.join(".config/chromium/NativeMessagingHosts"),
                Self::Edge => home.join(".config/microsoft-edge/NativeMessagingHosts"),
                Self::Firefox => home.join(".mozilla/native-messaging-hosts"),
            }
        }
    }
}

/// A Chromium extension id: 32 letters from a to p.
pub fn valid_chromium_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// A Firefox add-on id such as `name@domain`.
pub fn valid_firefox_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._-".contains(&b))
}

/// The manifest for one browser. `allowed` holds extension ids (Chromium
/// browsers) or add-on ids (Firefox); only these may talk to the host.
pub fn manifest(browser: Browser, host: &Path, allowed: &[String]) -> Value {
    let mut manifest = json!({
        "name": HOST_NAME,
        "description": "Mohona download manager: hands downloads from the browser to Mohona",
        "path": host.display().to_string(),
        "type": "stdio",
    });
    let map = manifest.as_object_mut().expect("an object");
    if browser.is_firefox() {
        map.insert("allowed_extensions".into(), json!(allowed));
    } else {
        let origins: Vec<String> = allowed
            .iter()
            .map(|id| format!("chrome-extension://{id}/"))
            .collect();
        map.insert("allowed_origins".into(), json!(origins));
    }
    manifest
}

/// What `install` did, for telling the person.
#[derive(Debug)]
pub struct Installed {
    pub browser: Browser,
    pub manifest: PathBuf,
    pub registry_key: Option<String>,
}

/// Where the places are: replaceable so tests can use a temporary folder.
#[derive(Debug, Clone)]
pub struct Places {
    pub home: PathBuf,
    /// Where manifests are kept on Windows (the registry points at them).
    pub data: PathBuf,
    pub windows: bool,
    pub mac: bool,
}

impl Places {
    pub fn current() -> Self {
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        let data = mohona_daemon::default_info_path()
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Self {
            home,
            data,
            windows: cfg!(windows),
            mac: cfg!(target_os = "macos"),
        }
    }

    pub fn manifest_path(&self, browser: Browser) -> PathBuf {
        if self.windows {
            self.data
                .join("native-messaging")
                .join(format!("{}.json", browser.name()))
        } else {
            browser
                .manifest_folder(&self.home, self.mac)
                .join(format!("{HOST_NAME}.json"))
        }
    }
}

fn write_manifest(path: &Path, manifest: &Value) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let text = serde_json::to_string_pretty(manifest).map_err(io::Error::other)?;
    std::fs::write(path, text + "\n")
}

#[cfg(windows)]
fn registry(args: &[&str]) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let status = std::process::Command::new("reg")
        .args(args)
        .creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| io::Error::other("the registry command failed"))
}

#[cfg(not(windows))]
fn registry(_: &[&str]) -> io::Result<()> {
    Ok(())
}

/// Registers the host with one browser.
pub fn install(
    places: &Places,
    browser: Browser,
    host: &Path,
    allowed: &[String],
) -> io::Result<Installed> {
    let path = places.manifest_path(browser);
    write_manifest(&path, &manifest(browser, host, allowed))?;
    let registry_key = if places.windows {
        let key = format!(r"HKCU\{}", browser.registry_key());
        registry(&[
            "add",
            &key,
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            &path.display().to_string(),
            "/f",
        ])?;
        Some(key)
    } else {
        None
    };
    Ok(Installed {
        browser,
        manifest: path,
        registry_key,
    })
}

/// Removes the registration. Returns false when there was none.
pub fn uninstall(places: &Places, browser: Browser) -> io::Result<bool> {
    let path = places.manifest_path(browser);
    let existed = path.exists();
    if existed {
        std::fs::remove_file(&path)?;
    }
    if places.windows {
        let key = format!(r"HKCU\{}", browser.registry_key());
        // Fails when the key is not there; that is fine.
        let _ = registry(&["delete", &key, "/f"]);
    }
    Ok(existed)
}

/// Is a manifest present, and which program and ids does it name?
pub fn status(places: &Places, browser: Browser) -> Option<(PathBuf, Vec<String>)> {
    let text = std::fs::read_to_string(places.manifest_path(browser)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let program = PathBuf::from(value.get("path")?.as_str()?);
    let key = if browser.is_firefox() {
        "allowed_extensions"
    } else {
        "allowed_origins"
    };
    let allowed = value
        .get(key)?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    Some((program, allowed))
}
