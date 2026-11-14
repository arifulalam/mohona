//! Looking for a newer Mohona on GitHub Releases.
//!
//! Mohona asks GitHub for the latest release of one repository (the person's
//! setting, `owner/repo`), compares version numbers, and gives the release
//! page's address. Installing is a separate step the person starts ("Install
//! now"): the installer of the release is downloaded, checked against the
//! checksum file published beside it (no checksum, no install), and only then
//! run. Only files of that repository's own releases are ever downloaded.

use mohona_net::HttpClient;

/// The setting that holds `owner/repo`.
pub const REPO_SETTING: &str = "update.repo";
/// The repository Mohona is published in, used until the person chooses another (or empties it to turn the check off).
pub const DEFAULT_REPO: &str = "arifulalam/mohona";
/// Where the GitHub API is (changed in tests).
pub const API: &str = "https://api.github.com";

/// What the latest release says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The version, without a leading `v`.
    pub version: String,
    /// The release's page.
    pub url: String,
    /// The release notes (cut to a sensible length).
    pub notes: String,
    /// The files attached to the release (only those that live in the repository's own releases).
    pub assets: Vec<Asset>,
}

/// A file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// `owner/repo` as GitHub names things: letters, digits, `.`, `_`, `-`, one slash.
pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    ok(owner) && ok(name)
}

/// `1.2.3`, `v1.2.3`, `1.2` or `1.2.3-beta.1` as numbers; a pre-release counts as its numbers
/// (the "latest" release of GitHub is never a pre-release, so this is only for the tag).
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let mut numbers = core.split('.');
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers.next().map_or(Some(0), |n| n.parse().ok())?;
    let patch = numbers.next().map_or(Some(0), |n| n.parse().ok())?;
    if numbers.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Is `latest` a higher version than `current`? Unreadable versions are never "newer".
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// Reads GitHub's answer for `releases/latest`.
pub fn parse_release(json: &str, repo: &str) -> Result<Release, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "GitHub's answer could not be read".to_string())?;
    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or("that repository has no release yet")?;
    if parse_version(tag).is_none() {
        return Err(format!(
            "the latest release is named \"{tag}\", which is not a version number"
        ));
    }
    // Only ever point at the repository's own pages.
    let own = format!("https://github.com/{repo}/");
    let url = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .filter(|u| {
            u.to_ascii_lowercase()
                .starts_with(&own.to_ascii_lowercase())
        })
        .map_or_else(
            || format!("https://github.com/{repo}/releases/latest"),
            str::to_string,
        );
    let notes = value.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let download_prefix =
        format!("https://github.com/{repo}/releases/download/").to_ascii_lowercase();
    let assets = value
        .get("assets")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let name = a.get("name")?.as_str()?.to_string();
                    let url = a.get("browser_download_url")?.as_str()?.to_string();
                    let plain_name = !name.is_empty()
                        && !name.contains(['/', '\\', ':'])
                        && name != "."
                        && name != "..";
                    (plain_name && url.to_ascii_lowercase().starts_with(&download_prefix)).then(
                        || Asset {
                            name,
                            url,
                            size: a.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version: tag.trim().trim_start_matches(['v', 'V']).to_string(),
        url,
        notes: notes.chars().take(2000).collect(),
        assets,
    })
}

/// The installer for this computer, and its checksum file, if the release has them.
/// Only Windows installs by itself (the NSIS "setup" program); other systems get `None`.
pub fn installer_for<'a>(assets: &'a [Asset], os: &str) -> Option<(&'a Asset, Option<&'a Asset>)> {
    let installer = match os {
        "windows" => assets.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            n.ends_with(".exe") && n.contains("setup")
        }),
        _ => None,
    }?;
    let wanted = format!("{}.sha256", installer.name.to_ascii_lowercase());
    let sum = assets
        .iter()
        .find(|a| a.name.to_ascii_lowercase() == wanted);
    Some((installer, sum))
}

/// The hash for `file` from a checksum file: `hash  name`, `hash *name`, or just the hash.
/// Lower-case hex of 64 characters, or `None`.
pub fn parse_checksum(text: &str, file: &str) -> Option<String> {
    let valid = |h: &str| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit());
    let mut only = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut parts = line.splitn(2, char::is_whitespace);
        let hash = parts.next()?;
        if !valid(hash) {
            continue;
        }
        match parts.next().map(|n| n.trim().trim_start_matches('*')) {
            Some(name) if name.eq_ignore_ascii_case(file) => {
                return Some(hash.to_ascii_lowercase())
            }
            Some(_) => {}
            None => only = Some(hash.to_ascii_lowercase()),
        }
    }
    only
}

/// Reads a small text file (a checksum) from the web.
pub async fn fetch_text(client: &HttpClient, url: &str) -> Result<String, String> {
    let mut body = client.get(url).await.map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    while let Some(chunk) = body.chunk().await.map_err(|e| e.to_string())? {
        data.extend_from_slice(&chunk);
        if data.len() > 1 << 16 {
            return Err("the checksum file is too big".into());
        }
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

/// Downloads `url` into `folder/name` and checks its SHA-256 against `expected` (lower-case hex).
/// A file that does not match is deleted. Returns the file's path.
pub async fn download_verified(
    client: &HttpClient,
    url: &str,
    folder: &std::path::Path,
    name: &str,
    expected: &str,
) -> Result<std::path::PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let path = folder.join(name);
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut body = client.get(url).await.map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let result: Result<(), String> = async {
        while let Some(chunk) = body.chunk().await.map_err(|e| e.to_string())? {
            total += chunk.len() as u64;
            if total > 1 << 30 {
                return Err("the installer is bigger than 1 GiB".to_string());
            }
            hasher.update(&chunk);
            file.write_all(&chunk).map_err(|e| e.to_string())?;
        }
        file.flush().map_err(|e| e.to_string())
    }
    .await;
    drop(file);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    let got: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != expected.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&path);
        return Err(
            "the downloaded installer does not match its checksum, so it was deleted".into(),
        );
    }
    Ok(path)
}

/// Starts the installer in the background so that it can replace this program. Windows only.
/// The flags are the ones the installer understands: passive (small progress window), update
/// (keep the settings) and relaunch (start Mohona again when done).
pub fn launch_installer(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;
        std::process::Command::new(path)
            .args(["/P", "/R", "/UPDATE"])
            .creation_flags(DETACHED)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not start the installer: {e}"))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("installing by itself only works on Windows; open the release page instead".into())
    }
}

/// Asks `api` (GitHub) for the latest release of `repo`.
pub async fn latest(client: &HttpClient, api: &str, repo: &str) -> Result<Release, String> {
    if !valid_repo(repo) {
        return Err("the repository must look like owner/name".into());
    }
    let url = format!("{}/repos/{repo}/releases/latest", api.trim_end_matches('/'));
    let mut body = client.get(&url).await.map_err(|e| {
        let text = e.to_string();
        if text.contains("404") {
            format!("{repo} has no published release (or it is private)")
        } else {
            format!("could not ask GitHub: {text}")
        }
    })?;
    let mut data = Vec::new();
    while let Some(chunk) = body.chunk().await.map_err(|e| e.to_string())? {
        data.extend_from_slice(&chunk);
        if data.len() > 1 << 20 {
            return Err("GitHub's answer was too big".into());
        }
    }
    parse_release(&String::from_utf8_lossy(&data), repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_as_numbers() {
        assert!(is_newer("0.0.1", "v0.0.2"));
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("1.2", "1.2.1"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("2.0.0", "1.9.9"));
        assert!(!is_newer("0.0.1", "nightly"));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn only_a_plain_owner_and_name_is_a_repository() {
        assert!(valid_repo("arifulalam/mohona"));
        assert!(valid_repo("a-b/c_d.e"));
        for bad in [
            "",
            "x",
            "a/b/c",
            "/b",
            "a/",
            "a/..",
            "a b/c",
            "a/b?x=1",
            "https://github.com/a/b",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
    }

    #[test]
    fn a_release_is_read_and_its_page_must_be_the_repositorys_own() {
        let json = r#"{"tag_name":"v1.4.0","html_url":"https://github.com/o/r/releases/tag/v1.4.0","body":"Fixes"}"#;
        let r = parse_release(json, "o/r").unwrap();
        assert_eq!((r.version.as_str(), r.notes.as_str()), ("1.4.0", "Fixes"));
        assert_eq!(r.url, "https://github.com/o/r/releases/tag/v1.4.0");
        let elsewhere = r#"{"tag_name":"1.0.0","html_url":"https://evil.test/x"}"#;
        assert_eq!(
            parse_release(elsewhere, "o/r").unwrap().url,
            "https://github.com/o/r/releases/latest"
        );
        assert!(parse_release(r#"{"message":"Not Found"}"#, "o/r").is_err());
        assert!(parse_release(r#"{"tag_name":"nightly"}"#, "o/r").is_err());
        assert!(parse_release("<html>", "o/r").is_err());
    }

    #[test]
    fn only_the_repositorys_own_release_files_are_offered() {
        let json = r#"{"tag_name":"v1.4.0","assets":[
          {"name":"Mohona_1.4.0_x64-setup.exe","browser_download_url":"https://github.com/o/r/releases/download/v1.4.0/Mohona_1.4.0_x64-setup.exe","size":9},
          {"name":"Mohona_1.4.0_x64-setup.exe.sha256","browser_download_url":"https://github.com/o/r/releases/download/v1.4.0/Mohona_1.4.0_x64-setup.exe.sha256","size":90},
          {"name":"evil.exe","browser_download_url":"https://evil.test/evil-setup.exe","size":1},
          {"name":"..\\x-setup.exe","browser_download_url":"https://github.com/o/r/releases/download/v1.4.0/x","size":1},
          {"name":"other.exe","browser_download_url":"https://github.com/p/q/releases/download/v1/other.exe","size":1}]}"#;
        let r = parse_release(json, "o/r").unwrap();
        assert_eq!(r.assets.len(), 2);
        let (installer, sum) = installer_for(&r.assets, "windows").unwrap();
        assert_eq!(installer.name, "Mohona_1.4.0_x64-setup.exe");
        assert_eq!(sum.unwrap().name, "Mohona_1.4.0_x64-setup.exe.sha256");
        assert!(installer_for(&r.assets, "linux").is_none());
    }

    #[test]
    fn a_checksum_is_read_in_the_usual_layouts() {
        let h = "ab".repeat(32);
        assert_eq!(
            parse_checksum(&format!("{h}  a.exe\n"), "a.exe"),
            Some(h.clone())
        );
        assert_eq!(
            parse_checksum(&format!("{h} *A.EXE"), "a.exe"),
            Some(h.clone())
        );
        assert_eq!(parse_checksum(&h, "a.exe"), Some(h.clone()));
        assert_eq!(parse_checksum(&format!("{h}  other.exe"), "a.exe"), None);
        assert_eq!(parse_checksum("not a hash  a.exe", "a.exe"), None);
        assert_eq!(parse_checksum(&"AB".repeat(32), "a.exe"), Some(h));
    }
}
