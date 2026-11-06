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

/// What to download and where.
#[derive(Clone, Debug)]
pub struct Request {
    pub url: String,
    pub folder: PathBuf,
    /// The ffmpeg program, if there is one: with it the best video and the best sound are joined;
    /// without it only the best single file (a lower quality) can be taken.
    pub ffmpeg: Option<PathBuf>,
}

/// The command line for yt-dlp.
pub fn arguments(request: &Request) -> Vec<String> {
    let mut args: Vec<String> = [
        "--no-playlist",
        "--no-colors",
        "--no-warnings",
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

/// What a failed run says, in a form a person can read: the last `ERROR:` line, without the prefix.
pub fn failure_reason(lines: &[String]) -> String {
    let error = lines
        .iter()
        .rev()
        .find_map(|l| l.trim().strip_prefix("ERROR:"))
        .map(|e| e.trim().to_string());
    match error {
        Some(e) if !e.is_empty() => {
            let hint = if e.contains("Sign in") || e.contains("confirm you") {
                " (the site wants a signed-in browser; yt-dlp could not get past it)"
            } else if e.contains("Requested format is not available") {
                " (try installing ffmpeg, so the best video and sound can be joined)"
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
    fn the_address_comes_after_the_end_of_options() {
        let with = arguments(&Request {
            url: "https://youtu.be/--exec=calc".into(),
            folder: PathBuf::from("D:\\Videos"),
            ffmpeg: Some(PathBuf::from("C:\\ff\\ffmpeg.exe")),
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
            failure_reason(&lines),
            "yt-dlp: [youtube] x: Video unavailable"
        );
        assert!(
            failure_reason(&["ERROR: Sign in to confirm you're not a bot".into()])
                .contains("signed-in browser")
        );
        assert!(failure_reason(&[]).contains("without saying why"));
    }
}
