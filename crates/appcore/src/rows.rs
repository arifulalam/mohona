use mohona_daemon::DownloadInfo;
use serde::Serialize;

/// One line of the download list, ready to show.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Row {
    pub id: i64,
    pub name: String,
    pub url: String,
    /// `queued`, `downloading`, `paused`, `completed`, `failed`, `cancelled` ...
    pub status: String,
    pub downloaded: u64,
    pub size: Option<u64>,
    /// 0–100 when the size is known.
    pub percent: Option<f64>,
    /// Bytes per second, measured between two lists; 0 when not downloading.
    pub speed: u64,
    /// Seconds left, when the speed and size are known.
    pub eta: Option<u64>,
    pub error: Option<String>,
    pub path: Option<String>,
    /// The link probably expired (see the browser extension's refresh).
    pub expired: bool,
}

/// A name for a download: its file name, else the last part of its address.
pub fn name_of(filename: Option<&str>, url: &str) -> String {
    if let Some(name) = filename.map(str::trim).filter(|n| !n.is_empty()) {
        return name.to_string();
    }
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let rest = without_query
        .split_once("://")
        .map_or(without_query, |(_, r)| r);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let last = path.rsplit('/').find(|p| !p.is_empty()).unwrap_or(host);
    let decoded = percent_decode(last);
    if decoded.is_empty() {
        host.to_string()
    } else {
        decoded
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) fn row(info: DownloadInfo, speed: u64) -> Row {
    let percent = info
        .size
        .filter(|s| *s > 0)
        .map(|s| (info.downloaded as f64 / s as f64 * 100.0).clamp(0.0, 100.0));
    let eta = match (info.size, speed) {
        (Some(size), s) if s > 0 && size > info.downloaded => {
            Some((size - info.downloaded).div_ceil(s))
        }
        _ => None,
    };
    Row {
        id: info.id,
        name: name_of(info.filename.as_deref(), &info.url),
        url: info.url,
        status: info.status,
        downloaded: info.downloaded,
        size: info.size,
        percent,
        speed,
        eta,
        error: info.error,
        path: info.path,
        expired: info.expired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_file_name_or_the_address() {
        assert_eq!(name_of(Some("a.zip"), "https://x.test/b"), "a.zip");
        assert_eq!(
            name_of(None, "https://x.test/dir/My%20File.zip?token=1#top"),
            "My File.zip"
        );
        assert_eq!(name_of(None, "https://x.test/"), "x.test");
        assert_eq!(name_of(Some("  "), "ftp://files.test/pub/"), "pub");
        assert_eq!(name_of(None, "https://x.test/%zz"), "%zz");
    }
}
