//! Video sites (YouTube and alike) through yt-dlp: which addresses are video pages, the command
//! line to run, and how to read what it prints. Running it is the caller's job.
//!
//! Everything a web page controls (the address) comes after `--`, so it can never become an option.

use std::path::{Path, PathBuf};

use url::Url;

/// Sites whose links are video *pages*: the real file is hidden behind a program that reads the page.
const SITES: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "vimeo.com",
    "dailymotion.com",
];

/// Whether `address` is a video page that needs yt-dlp (a single video, not a channel or a search).
pub fn is_site_page(address: &str) -> bool {
    let Ok(url) = Url::parse(address) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    if !SITES
        .iter()
        .any(|s| host == *s || host.ends_with(&format!(".{s}")))
    {
        return false;
    }
    let path = url.path();
    let query = |key: &str| url.query_pairs().any(|(k, v)| k == key && !v.is_empty());
    if host == "youtu.be" {
        return path.len() > 1;
    }
    if host.ends_with("youtube.com") || host.ends_with("youtube-nocookie.com") {
        return (path == "/watch" && query("v"))
            || path.starts_with("/shorts/")
            || path.starts_with("/live/")
            || path.starts_with("/embed/")
            || path.starts_with("/v/");
    }
    if host.ends_with("vimeo.com") {
        return path
            .trim_matches('/')
            .split('/')
            .next_back()
            .is_some_and(|last| !last.is_empty() && last.chars().all(|c| c.is_ascii_digit()));
    }
    // Dailymotion: /video/ID
    path.starts_with("/video/")
}

/// Whether `address` is an ordinary web page that *might* hold a video yt-dlp knows (Facebook,
/// X, Instagram, Pinterest, SoundCloud and a thousand more): http or https, not one of the sites
/// above (those are decided by [`is_site_page`]), and without a file name that says it is a file.
/// Mohona tries yt-dlp on such a page and, when yt-dlp does not know it, downloads it normally.
pub fn could_be_page(address: &str) -> bool {
    let Ok(url) = Url::parse(address) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    if SITES
        .iter()
        .any(|s| host == *s || host.ends_with(&format!(".{s}")))
    {
        return false;
    }
    let name = url.path().rsplit('/').next().unwrap_or("");
    match name.rsplit_once('.') {
        None => true,
        Some((_, ext)) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "html" | "htm" | "shtml" | "php" | "asp" | "aspx" | "jsp"
        ),
    }
}

/// The text of a cookie file (Netscape format) for the `Cookie` header of the browser page the
/// link came from. The cookies are tied to the page's own site, so yt-dlp sends them nowhere else
/// (unlike a `Cookie` header it would send to every address it visits). None when there is nothing
/// usable.
pub fn cookie_file_text(address: &str, header: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let domain = format!(".{}", host.strip_prefix("www.").unwrap_or(&host));
    let secure = if url.scheme() == "https" {
        "TRUE"
    } else {
        "FALSE"
    };
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    let mut any = false;
    for pair in header.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty()
            || name.contains(['\t', '\r', '\n'])
            || value.contains(['\t', '\r', '\n'])
        {
            continue;
        }
        text.push_str(&format!(
            "{domain}\tTRUE\t/\t{secure}\t0\t{name}\t{value}\n"
        ));
        any = true;
    }
    any.then_some(text)
}

/// The browsers yt-dlp can take cookies from (the names it knows).
pub const BROWSERS: &[&str] = &[
    "chrome", "edge", "firefox", "brave", "opera", "vivaldi", "chromium", "safari",
];

/// What to download and where.
#[derive(Clone, Debug)]
pub struct Request {
    pub url: String,
    pub folder: PathBuf,
    /// The ffmpeg program, if there is one: with it the best video and the best sound are joined;
    /// without it only the best single file (a lower quality) can be taken.
    pub ffmpeg: Option<PathBuf>,
    /// Take the cookies (the sign-in) of this browser, one of [`BROWSERS`].
    pub browser: Option<String>,
    /// A cookie file (Netscape format, see [`cookie_file_text`]) with the sign-in of the page the
    /// link came from.
    pub cookie_file: Option<PathBuf>,
    /// Let yt-dlp's "generic" reader look for a video inside any page. Off when Mohona is only
    /// guessing that a page holds a video, so an unknown page is refused quickly.
    pub generic: bool,
}

/// The command line for yt-dlp.
pub fn arguments(request: &Request) -> Vec<String> {
    let mut args: Vec<String> = [
        "--no-playlist",
        "--no-colors",
        "--newline",
        "--progress",
        "--no-mtime",
        "--windows-filenames",
        "--no-update",
        "--progress-template",
        "download:MOHONA_PROGRESS %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s",
        "--print",
        "before_dl:MOHONA_NAME %(title)s",
        "--print",
        "after_move:MOHONA_FILE %(filepath)s",
        "--no-quiet",
        "-o",
        "%(title).150B [%(id)s].%(ext)s",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    args.push("-P".into());
    args.push(request.folder.display().to_string());
    if !request.generic {
        args.extend(["--use-extractors".into(), "default,-generic".into()]);
    }
    if let Some(browser) = request.browser.as_deref().filter(|b| BROWSERS.contains(b)) {
        args.extend(["--cookies-from-browser".into(), browser.to_string()]);
    }
    if let Some(file) = &request.cookie_file {
        args.extend(["--cookies".into(), file.display().to_string()]);
    }
    match &request.ffmpeg {
        Some(ffmpeg) => {
            args.extend(["-f".into(), "bv*+ba/b".into()]);
            args.extend(["--merge-output-format".into(), "mp4".into()]);
            args.push("--ffmpeg-location".into());
            args.push(ffmpeg.display().to_string());
        }
        None => args.extend(["-f".into(), "b".into()]),
    }
    args.push("--".into());
    args.push(request.url.clone());
    args
}

/// One line yt-dlp printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// The video's title, when the download of a part begins.
    Name(String),
    /// Bytes downloaded of the current part, and its size when yt-dlp knows or estimates it.
    Progress { downloaded: u64, total: Option<u64> },
    /// The finished file.
    File(PathBuf),
    /// Anything else (yt-dlp's own messages).
    Other(String),
}

pub fn parse_line(line: &str) -> Line {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("MOHONA_PROGRESS ") {
        let mut it = rest.split_whitespace();
        let number = |s: Option<&str>| {
            s.and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| v as u64)
        };
        let downloaded = number(it.next()).unwrap_or(0);
        let total = number(it.next()).or_else(|| number(it.next()));
        return Line::Progress { downloaded, total };
    }
    if let Some(name) = line.strip_prefix("MOHONA_NAME ") {
        return Line::Name(name.trim().to_string());
    }
    if let Some(path) = line.strip_prefix("MOHONA_FILE ") {
        return Line::File(PathBuf::from(path.trim()));
    }
    Line::Other(line.to_string())
}

/// Whether yt-dlp said it does not know this page (so there is no video to take from it).
pub fn is_unsupported(lines: &[String]) -> bool {
    lines
        .iter()
        .any(|l| l.contains("Unsupported URL") || l.contains("is not a valid URL"))
}

/// What a failed run says, in a form a person can read: the last `ERROR:` line, without the prefix.
pub fn failure_reason(lines: &[String], has_ffmpeg: bool) -> String {
    let error = lines
        .iter()
        .rev()
        .find_map(|l| l.trim().strip_prefix("ERROR:"))
        .map(|e| e.trim().to_string());
    match error {
        Some(e) if !e.is_empty() => {
            let hint = if e.contains("Unsupported URL") {
                " (there is no video on this page that yt-dlp knows)"
            } else if e.contains("Sign in") || e.contains("confirm you") || e.contains("login") {
                " (the site wants a signed-in browser: choose your browser in Settings, Tools, so yt-dlp can use its sign-in)"
            } else if e.to_ascii_lowercase().contains("cookie") {
                " (close the browser and try again, or choose another browser in Settings, Tools)"
            } else if e.contains("Requested format is not available") {
                // Almost always one of three things; yt-dlp's own warnings say which.
                let said = |words: &[&str]| {
                    lines.iter().any(|l| {
                        let l = l.to_ascii_lowercase();
                        words.iter().any(|w| l.contains(w))
                    })
                };
                if said(&[
                    "javascript runtime",
                    "js runtime",
                    "challenge",
                    "nsig",
                    "signature solving",
                ]) {
                    " (YouTube needs a JavaScript runtime for yt-dlp: install Deno, in a terminal: winget install DenoLand.Deno, then restart Mohona)"
                } else if !has_ffmpeg {
                    " (ffmpeg was not found, so the best video and sound cannot be joined: see Settings, Tools)"
                } else {
                    " (this is usually an old yt-dlp, which YouTube outdates often: update it with  yt-dlp -U  or download the newest one)"
                }
            } else {
                ""
            };
            format!("yt-dlp: {e}{hint}")
        }
        _ => "yt-dlp stopped without saying why".to_string(),
    }
}

/// The place `yt-dlp` writes to, for a message.
pub fn folder_of(path: &Path) -> PathBuf {
    path.parent().map_or_else(PathBuf::new, Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_single_video_pages_of_known_sites_are_taken() {
        for yes in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?v=abc&list=PL1",
            "https://m.youtube.com/watch?v=abc",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/abc123",
            "https://www.youtube.com/live/abc123",
            "https://vimeo.com/76979871",
            "https://www.dailymotion.com/video/x7tgad0",
        ] {
            assert!(is_site_page(yes), "{yes}");
        }
        for no in [
            "https://www.youtube.com/",
            "https://www.youtube.com/@someone",
            "https://www.youtube.com/results?search_query=x",
            "https://www.youtube.com/playlist?list=PL1",
            "https://youtu.be/",
            "https://vimeo.com/channels/staffpicks",
            "https://example.com/watch?v=abc",
            "https://youtube.com.evil.example/watch?v=abc",
            "ftp://youtube.com/watch?v=abc",
            "magnet:?xt=urn:btih:abc",
            "not a url",
        ] {
            assert!(!is_site_page(no), "{no}");
        }
    }

    #[test]
    fn other_pages_are_tried_but_files_and_known_sites_are_not() {
        for yes in [
            "https://www.facebook.com/watch/?v=123",
            "https://x.com/someone/status/1234567890",
            "https://www.instagram.com/reel/abc/",
            "https://pin.it/abc",
            "https://soundcloud.com/artist/track",
            "https://example.com/page.html",
            "https://example.com/",
        ] {
            assert!(could_be_page(yes), "{yes}");
        }
        for no in [
            "https://example.com/file.zip",
            "https://example.com/movie.mp4",
            "https://www.youtube.com/@someone",
            "https://vimeo.com/channels/staffpicks",
            "ftp://example.com/page",
            "magnet:?xt=urn:btih:abc",
            "not a url",
        ] {
            assert!(!could_be_page(no), "{no}");
        }
    }

    #[test]
    fn browser_cookies_and_the_generic_reader_are_options() {
        let args = arguments(&Request {
            url: "https://x.com/a/status/1".into(),
            folder: PathBuf::from("D:\\V"),
            ffmpeg: None,
            browser: Some("edge".into()),
            cookie_file: Some(PathBuf::from("D:\\tmp\\c.txt")),
            generic: false,
        });
        assert!(args
            .windows(2)
            .any(|w| w == ["--cookies-from-browser", "edge"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--cookies", "D:\\tmp\\c.txt"]));
        assert!(args
            .windows(2)
            .any(|w| w == ["--use-extractors", "default,-generic"]));
        let none = arguments(&Request {
            url: "https://x.com/a/status/1".into(),
            folder: PathBuf::from("D:\\V"),
            ffmpeg: None,
            browser: Some("--exec=calc".into()),
            cookie_file: None,
            generic: true,
        });
        assert!(!none
            .iter()
            .any(|a| a.contains("exec") && a != none.last().unwrap()));
        assert!(!none.contains(&"--cookies".to_string()));
        assert!(!none.contains(&"--use-extractors".to_string()));
    }

    #[test]
    fn a_cookie_header_becomes_a_cookie_file_for_the_pages_own_site() {
        let text = cookie_file_text(
            "https://www.instagram.com/reel/abc/",
            "sessionid=abc; csrftoken=x y",
        )
        .unwrap();
        assert!(text.starts_with("# Netscape HTTP Cookie File\n"));
        assert!(
            text.contains(".instagram.com\tTRUE\t/\tTRUE\t0\tsessionid\tabc\n"),
            "{text}"
        );
        assert!(text.contains("csrftoken\tx y\n"));
        assert_eq!(cookie_file_text("https://x.com/a", ""), None);
        assert_eq!(cookie_file_text("https://x.com/a", "novalue"), None);
        assert_eq!(cookie_file_text("not a url", "a=b"), None);
    }

    #[test]
    fn an_unknown_page_is_told_apart_from_other_failures() {
        let unsupported = vec!["ERROR: Unsupported URL: https://example.com/".to_string()];
        assert!(is_unsupported(&unsupported));
        assert!(failure_reason(&unsupported, true).contains("no video on this page"));
        assert!(!is_unsupported(&["ERROR: Video unavailable".to_string()]));
    }

    #[test]
    fn the_address_comes_after_the_end_of_options() {
        let with = arguments(&Request {
            url: "https://youtu.be/--exec=calc".into(),
            folder: PathBuf::from("D:\\Videos"),
            ffmpeg: Some(PathBuf::from("C:\\ff\\ffmpeg.exe")),
            browser: None,
            cookie_file: None,
            generic: true,
        });
        assert_eq!(with[with.len() - 2], "--");
        assert_eq!(with.last().unwrap(), "https://youtu.be/--exec=calc");
        assert!(with.windows(2).any(|w| w == ["-f", "bv*+ba/b"]));
        assert!(with
            .windows(2)
            .any(|w| w == ["--ffmpeg-location", "C:\\ff\\ffmpeg.exe"]));
        assert!(with.windows(2).any(|w| w == ["-P", "D:\\Videos"]));
        assert!(with.contains(&"--no-playlist".to_string()));
        let without = arguments(&Request {
            url: "https://youtu.be/x".into(),
            folder: PathBuf::from("."),
            ffmpeg: None,
            browser: None,
            cookie_file: None,
            generic: true,
        });
        assert!(without.windows(2).any(|w| w == ["-f", "b"]));
        assert!(!without.contains(&"--merge-output-format".to_string()));
    }

    #[test]
    fn the_lines_are_read() {
        assert_eq!(
            parse_line("MOHONA_PROGRESS 1048576 NA 2097152.5"),
            Line::Progress {
                downloaded: 1_048_576,
                total: Some(2_097_152)
            }
        );
        assert_eq!(
            parse_line("MOHONA_PROGRESS 500 1000 NA"),
            Line::Progress {
                downloaded: 500,
                total: Some(1000)
            }
        );
        assert_eq!(
            parse_line("MOHONA_PROGRESS NA NA NA"),
            Line::Progress {
                downloaded: 0,
                total: None
            }
        );
        assert_eq!(
            parse_line("MOHONA_NAME My video "),
            Line::Name("My video".into())
        );
        assert_eq!(
            parse_line("MOHONA_FILE D:\\V\\My video [abc].mp4"),
            Line::File(PathBuf::from("D:\\V\\My video [abc].mp4"))
        );
        assert_eq!(
            parse_line("[youtube] abc: Downloading"),
            Line::Other("[youtube] abc: Downloading".into())
        );
    }

    #[test]
    fn a_failure_says_what_yt_dlp_said() {
        let lines = vec![
            "[youtube] x".to_string(),
            "ERROR: [youtube] x: Video unavailable".to_string(),
        ];
        assert_eq!(
            failure_reason(&lines, true),
            "yt-dlp: [youtube] x: Video unavailable"
        );
        assert!(
            failure_reason(&["ERROR: Sign in to confirm you're not a bot".into()], true)
                .contains("signed-in browser")
        );
        assert!(failure_reason(&[], true).contains("without saying why"));
    }

    #[test]
    fn an_unavailable_format_is_explained_by_what_is_really_missing() {
        let error =
            "ERROR: [youtube] x: Requested format is not available. Use --list-formats".to_string();
        let plain = vec![error.clone()];
        // ffmpeg was found: it is not blamed.
        let with = failure_reason(&plain, true);
        assert!(!with.contains("ffmpeg"), "{with}");
        assert!(with.contains("yt-dlp -U"), "{with}");
        assert!(failure_reason(&plain, false).contains("ffmpeg was not found"));
        // yt-dlp's own warning about the JavaScript runtime wins.
        let js = vec![
            "WARNING: [youtube] x: No supported JavaScript runtime could be found.".to_string(),
            error,
        ];
        assert!(failure_reason(&js, true).contains("Deno"));
        assert!(failure_reason(&js, false).contains("Deno"));
    }
}
