//! The site grabber: starts at a web page, lists the files it links to (pictures, videos, documents,
//! archives...) and, to the depth asked, follows the links to other pages and lists theirs.
//!
//! It only looks: choosing what to download, and downloading, are the caller's. It is polite: one
//! request at a time, a pause between requests, `robots.txt` obeyed unless switched off, and hard
//! limits on pages and on the size of a page.

mod html;
mod robots;

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use mohona_core::CancelToken;
use mohona_net::HttpClient;
use url::Url;

pub use html::{links, Kind, Link};
pub use robots::Robots;

/// Most pages read in one run, whatever the options say.
pub const MAX_PAGES: usize = 500;
/// Largest depth.
pub const MAX_DEPTH: u32 = 5;
/// A page is read up to this many bytes.
const PAGE_LIMIT: usize = 4 * 1024 * 1024;

/// What to look for.
#[derive(Clone, Debug)]
pub struct Options {
    /// 0: only the first page; 1: also the pages it links to; and so on (at most [`MAX_DEPTH`]).
    pub depth: u32,
    /// Follow links to pages of the same site only (the same host, `www.` ignored). Files are
    /// listed wherever they are.
    pub same_site: bool,
    /// Kinds of files to list: group names (`images`, `video`, `audio`, `documents`, `archives`,
    /// `programs`) or endings (`pdf`, `.pdf`). Empty: every file.
    pub types: Vec<String>,
    /// Most pages to read (at most [`MAX_PAGES`]).
    pub max_pages: usize,
    /// Pause between requests.
    pub delay: Duration,
    /// Obey `robots.txt`.
    pub robots: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            depth: 1,
            same_site: true,
            types: Vec::new(),
            max_pages: 100,
            delay: Duration::from_millis(300),
            robots: true,
        }
    }
}

/// A file found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub url: String,
    /// The name it would be saved under (from the address).
    pub name: String,
    /// Its group: `images`, `video`, `audio`, `documents`, `archives`, `programs` or `other`.
    pub group: &'static str,
    /// The page that links to it.
    pub page: String,
}

/// How far a run has got.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub pages: usize,
    pub files: usize,
    /// The page being read.
    pub current: String,
}

/// The result of a run.
#[derive(Clone, Debug, Default)]
pub struct Grabbed {
    pub files: Vec<Found>,
    pub pages: usize,
    /// Pages `robots.txt` asked not to visit.
    pub skipped_by_robots: usize,
    /// Pages that could not be read (other than the first).
    pub failed_pages: usize,
    /// Why it stopped early, if it did.
    pub note: Option<String>,
}

const IMAGES: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "tif", "tiff", "ico", "avif", "heic",
];
const VIDEO: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "3gp",
];
const AUDIO: &[&str] = &["mp3", "wav", "flac", "aac", "ogg", "m4a", "wma", "opus"];
const DOCUMENTS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "rtf", "odt", "ods", "odp", "epub",
    "csv",
];
const ARCHIVES: &[&str] = &["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "tgz", "iso"];
const PROGRAMS: &[&str] = &["exe", "msi", "dmg", "apk", "deb", "rpm", "appimage", "pkg"];
/// Endings of pages (and of the web's own machinery): followed, never listed.
const PAGES: &[&str] = &[
    "html", "htm", "xhtml", "shtml", "php", "asp", "aspx", "jsp", "cgi",
];
const JUNK: &[&str] = &["css", "js", "map", "woff", "woff2", "ttf", "eot", "otf"];

/// The group of an ending.
pub fn group_of(extension: &str) -> &'static str {
    let e = extension;
    for (name, list) in [
        ("images", IMAGES),
        ("video", VIDEO),
        ("audio", AUDIO),
        ("documents", DOCUMENTS),
        ("archives", ARCHIVES),
        ("programs", PROGRAMS),
    ] {
        if list.contains(&e) {
            return name;
        }
    }
    "other"
}

/// The ending of an address (lower case, 1 to 8 letters or digits), if its last part has one.
pub fn extension_of(url: &Url) -> Option<String> {
    let last = url.path().rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| ext.to_ascii_lowercase())
}

/// A name for the file an address points to.
pub fn name_of(url: &Url) -> String {
    let last = url.path().rsplit('/').find(|s| !s.is_empty()).unwrap_or("");
    let decoded = percent_decode(last);
    let clean: String = decoded
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"|?*\\/".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    if clean.is_empty() {
        url.host_str().unwrap_or("file").to_string()
    } else {
        clean
    }
}

fn percent_decode(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The endings the `types` ask for (empty: no filter).
pub fn wanted_endings(types: &[String]) -> HashSet<String> {
    let mut set = HashSet::new();
    for t in types {
        let t = t.trim().trim_start_matches('.').to_ascii_lowercase();
        let list: &[&str] = match t.as_str() {
            "images" | "image" | "pictures" => IMAGES,
            "video" | "videos" => VIDEO,
            "audio" | "music" | "sounds" => AUDIO,
            "documents" | "document" => DOCUMENTS,
            "archives" | "archive" => ARCHIVES,
            "programs" | "program" => PROGRAMS,
            _ => {
                if !t.is_empty() {
                    set.insert(t);
                }
                continue;
            }
        };
        set.extend(list.iter().map(|s| s.to_string()));
    }
    set
}

fn same_site(a: &Url, b: &Url) -> bool {
    let host = |u: &Url| {
        u.host_str()
            .map(|h| h.to_ascii_lowercase())
            .map(|h| h.strip_prefix("www.").map(str::to_string).unwrap_or(h))
    };
    host(a) == host(b)
}

/// What a link is, given the options.
enum Verdict {
    File(&'static str),
    Page,
    Ignore,
}

fn judge(link: &Link, wanted: &HashSet<String>) -> Verdict {
    let ext = extension_of(&link.url);
    let is_page_ending = ext.as_deref().is_none_or(|e| PAGES.contains(&e));
    let file = match link.kind {
        Kind::Frame => false,
        Kind::Download | Kind::Media => true,
        Kind::Anchor => !is_page_ending,
    };
    if !file {
        return if is_page_ending {
            Verdict::Page
        } else {
            Verdict::Ignore
        };
    }
    if link.kind == Kind::Anchor && ext.as_deref().is_some_and(|e| JUNK.contains(&e)) {
        return Verdict::Ignore;
    }
    let ext = ext.unwrap_or_default();
    if !wanted.is_empty() && !wanted.contains(&ext) {
        return Verdict::Ignore;
    }
    Verdict::File(group_of(&ext))
}

/// Reads up to [`PAGE_LIMIT`] bytes of a page as text. `None` when the address is not a web page
/// (its content type says it is a file); the second value is the address after redirects.
async fn fetch_page(
    client: &HttpClient,
    url: &Url,
) -> Result<(Option<String>, Url), mohona_net::NetError> {
    let mut body = client.get(url.as_str()).await?;
    let final_url = body
        .final_url()
        .and_then(|u| Url::parse(&u).ok())
        .unwrap_or_else(|| url.clone());
    if let Some(kind) = body.content_type() {
        if !(kind.contains("html") || kind == "text/plain" || kind.is_empty()) {
            return Ok((None, final_url));
        }
    }
    let mut data = Vec::new();
    while let Some(chunk) = body.chunk().await? {
        data.extend_from_slice(&chunk);
        if data.len() >= PAGE_LIMIT {
            break;
        }
    }
    Ok((Some(String::from_utf8_lossy(&data).into_owned()), final_url))
}

/// Looks through the site from `start`. `progress` is called after each page. Cancelling returns
/// what was found so far, with a note. The first page failing is an error; later ones are counted.
pub async fn grab(
    client: &HttpClient,
    start: &str,
    options: &Options,
    cancel: &CancelToken,
    mut progress: impl FnMut(Progress) + Send,
) -> Result<Grabbed, String> {
    let start = Url::parse(start.trim()).map_err(|_| "that is not a web address".to_string())?;
    if !matches!(start.scheme(), "http" | "https") {
        return Err("the site grabber reads http and https addresses".into());
    }
    let depth = options.depth.min(MAX_DEPTH);
    let max_pages = options.max_pages.clamp(1, MAX_PAGES);
    let wanted = wanted_endings(&options.types);

    let mut result = Grabbed::default();
    let mut seen_pages: HashSet<String> = HashSet::new();
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut robots: HashMap<String, Robots> = HashMap::new();
    let mut queue: VecDeque<(Url, u32)> = VecDeque::new();
    seen_pages.insert(start.as_str().to_string());
    queue.push_back((start.clone(), 0));
    let mut first = true;

    while let Some((url, level)) = queue.pop_front() {
        if cancel.is_cancelled() {
            result.note = Some("stopped".into());
            break;
        }
        if result.pages >= max_pages {
            result.note = Some(format!(
                "stopped after {max_pages} pages (there are more: raise the limit or lower the depth)"
            ));
            break;
        }
        // robots.txt of this site, read once.
        let mut pause = options.delay;
        if options.robots {
            let origin = format!(
                "{}://{}",
                url.scheme(),
                url.host_str().unwrap_or("").to_ascii_lowercase()
            ) + &url.port().map(|p| format!(":{p}")).unwrap_or_default();
            if !robots.contains_key(&origin) {
                let text = match client.get(&format!("{origin}/robots.txt")).await {
                    Ok(mut body) => {
                        let mut data = Vec::new();
                        while let Ok(Some(chunk)) = body.chunk().await {
                            data.extend_from_slice(&chunk);
                            if data.len() > 512 * 1024 {
                                break;
                            }
                        }
                        String::from_utf8_lossy(&data).into_owned()
                    }
                    Err(_) => String::new(),
                };
                robots.insert(origin.clone(), Robots::parse(&text));
            }
            let rules = &robots[&origin];
            let path = match url.query() {
                Some(q) => format!("{}?{q}", url.path()),
                None => url.path().to_string(),
            };
            if !rules.allows(&path) {
                result.skipped_by_robots += 1;
                continue;
            }
            if let Some(d) = rules.crawl_delay {
                pause = pause.max(Duration::from_secs_f64(d.min(5.0)));
            }
        }
        if !first {
            tokio::select! {
                () = cancel.cancelled() => {
                    result.note = Some("stopped".into());
                    break;
                }
                () = tokio::time::sleep(pause) => {}
            }
        }
        let fetched = tokio::select! {
            () = cancel.cancelled() => {
                result.note = Some("stopped".into());
                break;
            }
            fetched = fetch_page(client, &url) => fetched,
        };
        let (text, final_url) = match fetched {
            Ok(v) => v,
            Err(e) => {
                if first {
                    return Err(format!("{url} could not be read: {e}"));
                }
                result.failed_pages += 1;
                continue;
            }
        };
        first = false;
        result.pages += 1;
        let Some(text) = text else {
            // The address is a file after all (the page's content type said so).
            if seen_files.insert(url.as_str().to_string()) {
                let ext = extension_of(&url).unwrap_or_default();
                if wanted.is_empty() || wanted.contains(&ext) {
                    result.files.push(Found {
                        name: name_of(&url),
                        group: group_of(&ext),
                        url: url.to_string(),
                        page: url.to_string(),
                    });
                }
            }
            continue;
        };
        for link in links(&text, &final_url) {
            match judge(&link, &wanted) {
                Verdict::File(group) => {
                    if seen_files.insert(link.url.as_str().to_string()) {
                        result.files.push(Found {
                            name: name_of(&link.url),
                            group,
                            url: link.url.to_string(),
                            page: final_url.to_string(),
                        });
                    }
                }
                Verdict::Page => {
                    if level < depth
                        && (!options.same_site || same_site(&link.url, &start))
                        && seen_pages.insert(link.url.as_str().to_string())
                    {
                        queue.push_back((link.url, level + 1));
                    }
                }
                Verdict::Ignore => {}
            }
        }
        progress(Progress {
            pages: result.pages,
            files: result.files.len(),
            current: url.to_string(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_endings_and_groups() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert_eq!(
            name_of(&u("https://a.com/x/My%20File.pdf?x=1")),
            "My File.pdf"
        );
        assert_eq!(name_of(&u("https://a.com/")), "a.com");
        assert_eq!(
            extension_of(&u("https://a.com/a.tar.GZ")).as_deref(),
            Some("gz")
        );
        assert_eq!(extension_of(&u("https://a.com/dir/")), None);
        assert_eq!(extension_of(&u("https://a.com/a.verylongending")), None);
        assert_eq!(group_of("pdf"), "documents");
        assert_eq!(group_of("xyz"), "other");
    }

    #[test]
    fn types_mix_groups_and_endings() {
        let w = wanted_endings(&["images".into(), ".PDF".into(), " ".into()]);
        assert!(w.contains("jpg") && w.contains("pdf") && !w.contains("mp4"));
        assert!(wanted_endings(&[]).is_empty());
    }

    #[test]
    fn only_the_same_site_is_followed_when_asked() {
        let a = Url::parse("https://www.example.com/a").unwrap();
        assert!(same_site(&a, &Url::parse("https://example.com/b").unwrap()));
        assert!(!same_site(
            &a,
            &Url::parse("https://blog.example.com/b").unwrap()
        ));
    }
}
