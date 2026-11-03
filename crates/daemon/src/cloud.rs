//! Share links of cloud storage and code hosts, turned into direct download addresses.
//!
//! A link such as `https://drive.google.com/file/d/ID/view` opens a web page,
//! not the file. This module knows, for a few services, which address gives
//! the file itself. Only exact host names are rewritten (never look-alikes),
//! identifiers are checked for the characters the services use, and anything
//! not recognised is left alone: `resolve` then returns `None`.

use base64::Engine as _;

/// A direct address for a share link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The address that gives the file.
    pub url: String,
    /// The service the link was for: `Google Drive`, `Google Docs`, `Dropbox`, `OneDrive` or `GitHub`.
    pub service: &'static str,
}

fn link(url: String, service: &'static str) -> Option<Link> {
    Some(Link { url, service })
}

/// An address split into its host, path segments and query text (web addresses only).
fn parts(url: &str) -> Option<(String, Vec<&str>, &str)> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let host = crate::rules::host_of(url)?;
    let rest = rest.split('#').next()?;
    let tail = &rest[rest.find(['/', '?']).unwrap_or(rest.len())..];
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    Some((
        host,
        path.split('/').filter(|s| !s.is_empty()).collect(),
        query,
    ))
}

/// An identifier as these services write them.
fn id_ok(id: &str) -> bool {
    (10..=200).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// The direct address for a share link, if it is one of the known kinds.
pub fn resolve(url: &str) -> Option<Link> {
    let (host, path, query) = parts(url)?;
    match host.as_str() {
        "drive.google.com" => drive(&path, query),
        "docs.google.com" => docs(&path),
        "www.dropbox.com" | "dropbox.com" => dropbox(url, &path),
        "1drv.ms" | "onedrive.live.com" => onedrive(url),
        "github.com" => github(&path),
        _ => None,
    }
}

fn drive(path: &[&str], query: &str) -> Option<Link> {
    // /file/d/ID/view, /file/u/0/d/ID/view, /open?id=ID, /uc?id=ID, /uc?export=download&id=ID
    let id = match path {
        ["file", "d", id, ..] => Some(*id),
        ["file", "u", _, "d", id, ..] => Some(*id),
        ["open"] | ["uc"] => query_value(query, "id"),
        _ => None,
    }?;
    id_ok(id).then_some(())?;
    // "confirm=t" skips the page that warns that a large file cannot be scanned.
    link(
        format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t"),
        "Google Drive",
    )
}

fn docs(path: &[&str]) -> Option<Link> {
    // Documents, sheets and slides are exported as Office files.
    let (kind, id) = match path {
        [kind @ ("document" | "spreadsheets" | "presentation"), "d", id, ..] => (*kind, *id),
        _ => return None,
    };
    id_ok(id).then_some(())?;
    let tail = match kind {
        "document" => "export?format=docx",
        "spreadsheets" => "export?format=xlsx",
        _ => "export/pptx",
    };
    link(
        format!("https://docs.google.com/{kind}/d/{id}/{tail}"),
        "Google Docs",
    )
}

fn dropbox(url: &str, path: &[&str]) -> Option<Link> {
    // /s/..., /scl/fi/... (needs its rlkey), /scl/fo/... and /sh/... are shared items.
    if !matches!(path.first(), Some(&("s" | "scl" | "sh"))) || path.len() < 2 {
        return None;
    }
    let (base, query) = url
        .trim()
        .split('#')
        .next()?
        .split_once('?')
        .unwrap_or((url.trim(), ""));
    let mut kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with("dl=") && !p.starts_with("raw="))
        .collect();
    kept.push("dl=1");
    let base = base.replacen("http://", "https://", 1);
    link(format!("{base}?{}", kept.join("&")), "Dropbox")
}

fn onedrive(url: &str) -> Option<Link> {
    // The sharing API takes the share link itself, base64url-encoded behind "u!".
    let clean = url.trim().split('#').next()?;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(clean.as_bytes());
    link(
        format!("https://api.onedrive.com/v1.0/shares/u!{encoded}/root/content"),
        "OneDrive",
    )
}

fn github(path: &[&str]) -> Option<Link> {
    // /OWNER/REPO/blob/REF/PATH... is the page of a file; raw.githubusercontent.com has the file.
    let [owner, repo, "blob", reference, file @ ..] = path else {
        return None;
    };
    if file.is_empty() {
        return None;
    }
    let name_ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    if !name_ok(owner) || !name_ok(repo) {
        return None;
    }
    if reference.is_empty() || reference.contains(['?', '#']) {
        return None;
    }
    link(
        format!(
            "https://raw.githubusercontent.com/{owner}/{repo}/{reference}/{}",
            file.join("/")
        ),
        "GitHub",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Option<String> {
        resolve(text).map(|l| l.url)
    }

    const ID: &str = "1AbCdEfGhIjKlMnOpQrStUvWxYz_0123-45";

    #[test]
    fn google_drive_links_of_every_common_shape() {
        let want = format!(
            "https://drive.usercontent.google.com/download?id={ID}&export=download&confirm=t"
        );
        for text in [
            format!("https://drive.google.com/file/d/{ID}/view?usp=sharing"),
            format!("https://drive.google.com/file/d/{ID}/edit"),
            format!("https://drive.google.com/file/u/0/d/{ID}/view"),
            format!("https://drive.google.com/open?id={ID}"),
            format!("https://drive.google.com/uc?export=download&id={ID}"),
            format!("http://drive.google.com/uc?id={ID}"),
        ] {
            assert_eq!(url(&text).as_deref(), Some(want.as_str()), "{text}");
        }
        assert_eq!(
            resolve(&format!("https://drive.google.com/file/d/{ID}/view"))
                .unwrap()
                .service,
            "Google Drive"
        );
        // Folders, short ids and odd characters are not files.
        for text in [
            format!("https://drive.google.com/drive/folders/{ID}"),
            "https://drive.google.com/file/d/short/view".to_string(),
            "https://drive.google.com/file/d/abc%2Fdef%2Fghijk/view".to_string(),
            "https://drive.google.com/open?id=".to_string(),
        ] {
            assert_eq!(url(&text), None, "{text}");
        }
    }

    #[test]
    fn google_docs_are_exported_as_office_files() {
        assert_eq!(
            url(&format!("https://docs.google.com/document/d/{ID}/edit")).unwrap(),
            format!("https://docs.google.com/document/d/{ID}/export?format=docx")
        );
        assert_eq!(
            url(&format!(
                "https://docs.google.com/spreadsheets/d/{ID}/edit#gid=0"
            ))
            .unwrap(),
            format!("https://docs.google.com/spreadsheets/d/{ID}/export?format=xlsx")
        );
        assert_eq!(
            url(&format!("https://docs.google.com/presentation/d/{ID}/edit")).unwrap(),
            format!("https://docs.google.com/presentation/d/{ID}/export/pptx")
        );
        assert_eq!(
            url(&format!("https://docs.google.com/forms/d/{ID}/edit")),
            None
        );
    }

    #[test]
    fn dropbox_links_ask_for_the_file_and_keep_their_keys() {
        assert_eq!(
            url("https://www.dropbox.com/s/abc123/report.pdf?dl=0").unwrap(),
            "https://www.dropbox.com/s/abc123/report.pdf?dl=1"
        );
        assert_eq!(
            url("https://www.dropbox.com/s/abc123/report.pdf").unwrap(),
            "https://www.dropbox.com/s/abc123/report.pdf?dl=1"
        );
        assert_eq!(
            url("https://www.dropbox.com/scl/fi/xyz/photo.jpg?rlkey=KEY&st=abc&dl=0").unwrap(),
            "https://www.dropbox.com/scl/fi/xyz/photo.jpg?rlkey=KEY&st=abc&dl=1"
        );
        assert_eq!(
            url("http://dropbox.com/s/abc/a.zip?raw=1&dl=0").unwrap(),
            "https://dropbox.com/s/abc/a.zip?dl=1"
        );
        assert_eq!(url("https://www.dropbox.com/home"), None);
        assert_eq!(url("https://www.dropbox.com/s"), None);
        assert_eq!(url("https://dl.dropboxusercontent.com/s/abc/a.zip"), None);
    }

    #[test]
    fn onedrive_share_links_go_through_the_sharing_api() {
        let l = resolve("https://1drv.ms/u/s!AbCdEf").unwrap();
        assert_eq!(l.service, "OneDrive");
        // "u!" + base64url of the link, no padding.
        let encoded =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("https://1drv.ms/u/s!AbCdEf");
        assert_eq!(
            l.url,
            format!("https://api.onedrive.com/v1.0/shares/u!{encoded}/root/content")
        );
        assert!(!l.url.contains('=') && !l.url.contains('+'));
        assert!(url("https://onedrive.live.com/?cid=1&id=2&authkey=3").is_some());
        assert_eq!(url("https://onedrive.evil.test/?cid=1"), None);
    }

    #[test]
    fn github_file_pages_become_raw_addresses() {
        assert_eq!(
            url("https://github.com/rust-lang/rust/blob/master/README.md?plain=1#L3").unwrap(),
            "https://raw.githubusercontent.com/rust-lang/rust/master/README.md"
        );
        assert_eq!(
            url("https://github.com/o/r/blob/v1.2/dir/sub/file.txt").unwrap(),
            "https://raw.githubusercontent.com/o/r/v1.2/dir/sub/file.txt"
        );
        assert_eq!(url("https://github.com/o/r/blob/main"), None);
        assert_eq!(url("https://github.com/o/r/tree/main/dir"), None);
        assert_eq!(
            url("https://github.com/o/r/releases/download/v1/a.zip"),
            None
        );
        assert_eq!(url("https://github.com/o%2F/r/blob/main/a.txt"), None);
    }

    #[test]
    fn look_alike_hosts_and_other_addresses_are_left_alone() {
        for text in [
            format!("https://drive.google.com.evil.test/file/d/{ID}/view"),
            format!("https://evil.test/drive.google.com/file/d/{ID}/view"),
            format!("https://notdrive.google.com/file/d/{ID}/view"),
            format!("https://user@drive.google.com.evil.test/file/d/{ID}/view"),
            format!("ftp://drive.google.com/file/d/{ID}/view"),
            "https://example.com/a.zip".to_string(),
            "magnet:?xt=urn:btih:abc".to_string(),
            "not an address".to_string(),
        ] {
            assert_eq!(url(&text), None, "{text}");
        }
    }
}
