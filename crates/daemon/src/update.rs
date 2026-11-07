//! Looking for a newer Mohona on GitHub Releases.
//!
//! Mohona only *looks*: it asks GitHub for the latest release of one
//! repository (the person's setting, `owner/repo`), compares version numbers,
//! and gives the release page's address. Installing is the person's step, from
//! that page; nothing is downloaded or run by this.

use mohona_net::HttpClient;

/// The setting that holds `owner/repo`.
pub const REPO_SETTING: &str = "update.repo";
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
    Ok(Release {
        version: tag.trim().trim_start_matches(['v', 'V']).to_string(),
        url,
        notes: notes.chars().take(2000).collect(),
    })
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
}
