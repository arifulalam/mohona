//! Files that only the browser has: a `blob:` or `data:` address holds its
//! bytes inside the page or the address itself, so there is nothing for the
//! engine to download. The extension reads the bytes and sends them over in
//! pieces; this module puts them together, works out what kind of file it
//! is, and saves it under a safe name that never replaces an existing file.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;

/// The most a single file may have in total.
pub const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
/// The most one piece may hold (decoded).
pub const MAX_PIECE: usize = 768 * 1024;
const MAX_TRANSFERS: usize = 4;
const IDLE: Duration = Duration::from_secs(600);

struct Transfer {
    file: File,
    temporary: PathBuf,
    folder: PathBuf,
    name: String,
    mime: Option<String>,
    written: u64,
    next_piece: u64,
    head: Vec<u8>,
    touched: Instant,
}

#[derive(Default)]
pub struct Receiver {
    transfers: Mutex<HashMap<u64, Transfer>>,
    next: Mutex<u64>,
}

/// Known kinds, found from the first bytes: (extension, check).
pub fn detect(head: &[u8]) -> Option<&'static str> {
    let starts = |p: &[u8]| head.starts_with(p);
    let at = |i: usize, p: &[u8]| head.get(i..i + p.len()) == Some(p);
    Some(if starts(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if starts(b"\xff\xd8\xff") {
        "jpg"
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        "gif"
    } else if starts(b"RIFF") && at(8, b"WEBP") {
        "webp"
    } else if starts(b"RIFF") && at(8, b"WAVE") {
        "wav"
    } else if starts(b"%PDF-") {
        "pdf"
    } else if starts(b"PK\x03\x04") {
        "zip"
    } else if starts(b"\x1f\x8b") {
        "gz"
    } else if starts(b"7z\xbc\xaf\x27\x1c") {
        "7z"
    } else if starts(b"Rar!\x1a\x07") {
        "rar"
    } else if at(4, b"ftyp") {
        if at(8, b"M4A ") {
            "m4a"
        } else {
            "mp4"
        }
    } else if starts(b"\x1a\x45\xdf\xa3") {
        "webm"
    } else if starts(b"OggS") {
        "ogg"
    } else if starts(b"fLaC") {
        "flac"
    } else if starts(b"ID3") || starts(b"\xff\xfb") || starts(b"\xff\xf3") {
        "mp3"
    } else if starts(b"BM") && head.len() > 14 {
        "bmp"
    } else if starts(b"\x00\x00\x01\x00") {
        "ico"
    } else if head
        .iter()
        .take(512)
        .copied()
        .collect::<Vec<u8>>()
        .windows(4)
        .any(|w| w == b"<svg")
    {
        "svg"
    } else {
        return None;
    })
}

/// The extension for a media type, when the bytes did not say.
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    Some(
        match mime.split(';').next()?.trim().to_ascii_lowercase().as_str() {
            "text/plain" => "txt",
            "text/html" => "html",
            "text/css" => "css",
            "text/csv" => "csv",
            "text/markdown" => "md",
            "application/json" => "json",
            "application/xml" | "text/xml" => "xml",
            "image/svg+xml" => "svg",
            "application/pdf" => "pdf",
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "application/zip" => "zip",
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            "audio/mpeg" => "mp3",
            "audio/ogg" => "ogg",
            "audio/wav" | "audio/x-wav" => "wav",
            _ => return None,
        },
    )
}

/// Extensions this module will replace when the bytes prove them wrong.
const REPLACEABLE: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "pdf", "zip", "gz", "7z", "rar",
    "mp4", "m4a", "webm", "ogg", "flac", "mp3", "wav", "txt", "html", "json", "xml", "csv", "bin",
];

/// Extensions that are the same kind as what was found (a `.docx` is a zip).
fn same_family(found: &str, ext: &str) -> bool {
    match found {
        "jpg" => ext == "jpeg" || ext == "jpg" || ext == "jfif",
        "zip" => [
            "zip", "docx", "xlsx", "pptx", "odt", "ods", "odp", "epub", "jar", "apk", "xpi",
        ]
        .contains(&ext),
        "mp4" => ["mp4", "m4v", "mov", "m4a"].contains(&ext),
        "webm" => ["webm", "mkv"].contains(&ext),
        "ogg" => ["ogg", "oga", "ogv", "opus"].contains(&ext),
        "gz" => ["gz", "tgz"].contains(&ext),
        other => other == ext,
    }
}

/// A name that is safe to create: no folders, nothing Windows refuses.
pub fn sanitize(name: &str) -> String {
    let mut clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    clean = clean
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" .", ".");
    clean = clean.trim_matches(|c| c == '.' || c == ' ').to_string();
    let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if clean.is_empty() || reserved {
        clean = format!("download {clean}").trim().to_string();
    }
    if clean.chars().count() > 120 {
        let ext = clean
            .rsplit_once('.')
            .map(|(_, e)| e)
            .filter(|e| e.len() <= 8);
        let keep: String = clean.chars().take(100).collect();
        clean = match ext {
            Some(e) => format!("{}.{e}", keep.trim_end_matches('.')),
            None => keep,
        };
    }
    clean
}

/// The name to save under: sanitized, with the extension of what the bytes
/// are (or the media type, for plain text and the like).
pub fn final_name(name: &str, head: &[u8], mime: Option<&str>) -> String {
    let name = sanitize(name);
    let found = detect(head).or_else(|| mime.and_then(extension_for_mime));
    let Some(found) = found else {
        return name;
    };
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => {
            let lower = ext.to_ascii_lowercase();
            if same_family(found, &lower) {
                name
            } else if REPLACEABLE.contains(&lower.as_str()) {
                format!("{stem}.{found}")
            } else {
                format!("{name}.{found}")
            }
        }
        _ => format!("{name}.{found}"),
    }
}

/// `name`, or `name (1).ext`, `name (2).ext`... whichever is free.
pub fn free_path(folder: &Path, name: &str) -> PathBuf {
    let candidate = folder.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..10_000)
        .map(|i| folder.join(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| folder.join(format!("{stem} (copy){ext}")))
}

impl Receiver {
    fn drop_idle(map: &mut HashMap<u64, Transfer>) {
        map.retain(|_, t| {
            let keep = t.touched.elapsed() < IDLE;
            if !keep {
                let _ = std::fs::remove_file(&t.temporary);
            }
            keep
        });
    }

    pub fn start(&self, folder: &Path, name: &str, mime: Option<String>) -> Result<u64, String> {
        let mut map = self.transfers.lock().map_err(|_| "internal error")?;
        Self::drop_idle(&mut map);
        if map.len() >= MAX_TRANSFERS {
            return Err("too many files are being received at once".into());
        }
        std::fs::create_dir_all(folder).map_err(|e| format!("cannot use the folder: {e}"))?;
        let id = {
            let mut next = self.next.lock().map_err(|_| "internal error")?;
            *next += 1;
            *next
        };
        let temporary = folder.join(format!(".mohona-receive-{}-{id}.part", std::process::id()));
        let file = File::create(&temporary).map_err(|e| format!("cannot create the file: {e}"))?;
        map.insert(
            id,
            Transfer {
                file,
                temporary,
                folder: folder.to_path_buf(),
                name: name.to_string(),
                mime,
                written: 0,
                next_piece: 0,
                head: Vec::new(),
                touched: Instant::now(),
            },
        );
        Ok(id)
    }

    fn append(t: &mut Transfer, index: u64, data: &str) -> Result<(), String> {
        if index != t.next_piece {
            return Err(format!(
                "piece {index} came when {} was expected",
                t.next_piece
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| "a piece is not valid base64".to_string())?;
        if bytes.len() > MAX_PIECE {
            return Err("a piece is larger than allowed".into());
        }
        if t.written + bytes.len() as u64 > MAX_TOTAL {
            return Err("the file is larger than the 1 GiB limit".into());
        }
        t.file
            .write_all(&bytes)
            .map_err(|e| format!("cannot write the file: {e}"))?;
        if t.head.len() < 600 {
            let room = 600 - t.head.len();
            t.head.extend_from_slice(&bytes[..bytes.len().min(room)]);
        }
        t.written += bytes.len() as u64;
        t.next_piece += 1;
        t.touched = Instant::now();
        Ok(())
    }

    /// Adds the next piece. Any error ends the transfer and deletes what was received.
    pub fn piece(&self, id: u64, index: u64, data: &str) -> Result<(), String> {
        let mut map = self.transfers.lock().map_err(|_| "internal error")?;
        let Some(t) = map.get_mut(&id) else {
            return Err(format!("no file {id} is being received"));
        };
        let result = Self::append(t, index, data);
        if result.is_err() {
            if let Some(t) = map.remove(&id) {
                let _ = std::fs::remove_file(&t.temporary);
            }
        }
        result
    }

    /// Finishes the file; returns where it is.
    pub fn finish(&self, id: u64) -> Result<PathBuf, String> {
        let mut map = self.transfers.lock().map_err(|_| "internal error")?;
        let Some(t) = map.remove(&id) else {
            return Err(format!("no file {id} is being received"));
        };
        let done = (|| {
            t.file.sync_all().map_err(|e| e.to_string())?;
            if t.written == 0 {
                return Err("the file is empty".to_string());
            }
            let name = final_name(&t.name, &t.head, t.mime.as_deref());
            // `hard_link` fails if the name is taken, so two files never collide.
            for _ in 0..50 {
                let target = free_path(&t.folder, &name);
                match std::fs::hard_link(&t.temporary, &target) {
                    Ok(()) => return Ok(target),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(_) => {
                        // Some file systems have no links: a rename is enough.
                        return std::fs::rename(&t.temporary, &target)
                            .map(|()| target)
                            .map_err(|e| e.to_string());
                    }
                }
            }
            Err("could not find a free file name".to_string())
        })();
        let _ = std::fs::remove_file(&t.temporary);
        done
    }

    pub fn abort(&self, id: u64) {
        if let Ok(mut map) = self.transfers.lock() {
            if let Some(t) = map.remove(&id) {
                let _ = std::fs::remove_file(&t.temporary);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_bytes_tell_what_a_file_is() {
        assert_eq!(detect(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(detect(b"\xff\xd8\xff\xe0"), Some("jpg"));
        assert_eq!(detect(b"%PDF-1.7"), Some("pdf"));
        assert_eq!(detect(b"PK\x03\x04xx"), Some("zip"));
        assert_eq!(detect(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(detect(b"RIFF\0\0\0\0WAVEfmt "), Some("wav"));
        assert_eq!(detect(b"\0\0\0\x18ftypmp42"), Some("mp4"));
        assert_eq!(detect(b"\0\0\0\x18ftypM4A "), Some("m4a"));
        assert_eq!(
            detect(b"<?xml version='1.0'?><svg xmlns='x'/>"),
            Some("svg")
        );
        assert_eq!(detect(b"hello world"), None);
        assert_eq!(detect(b""), None);
    }

    #[test]
    fn names_get_the_right_extension_and_nothing_dangerous() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert_eq!(final_name("photo", png, None), "photo.png");
        assert_eq!(final_name("photo.png", png, None), "photo.png");
        assert_eq!(final_name("photo.jpg", png, None), "photo.png");
        assert_eq!(
            final_name("report.docx", b"PK\x03\x04", None),
            "report.docx"
        );
        assert_eq!(final_name("v1.2", png, None), "v1.2.png");
        assert_eq!(
            final_name("notes", b"hello", Some("text/plain")),
            "notes.txt"
        );
        assert_eq!(
            final_name("notes", b"hello", Some("application/x-unknown")),
            "notes"
        );
        assert_eq!(final_name("../../etc/passwd", b"hello", None), "etc passwd");
        assert_eq!(final_name("a\r\nb:c*.png", png, None), "a b c.png");
        assert_eq!(final_name("CON", png, None), "download CON.png");
        assert_eq!(final_name("", png, None), "download.png");
        assert_eq!(final_name("...", png, None), "download.png");
        let long = "x".repeat(300);
        assert!(final_name(&long, png, None).chars().count() <= 106);
    }

    #[test]
    fn a_file_is_assembled_from_pieces_and_never_replaces_another() {
        let dir = tempfile::tempdir().unwrap();
        let r = Receiver::default();
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let save = |body: &[&[u8]]| {
            let id = r.start(dir.path(), "pic", None).unwrap();
            for (i, part) in body.iter().enumerate() {
                r.piece(id, i as u64, &b64(part)).unwrap();
            }
            r.finish(id).unwrap()
        };
        let first = save(&[b"\x89PNG\r\n\x1a\n", b"more"]);
        assert_eq!(first.file_name().unwrap(), "pic.png");
        assert_eq!(std::fs::read(&first).unwrap(), b"\x89PNG\r\n\x1a\nmore");
        let second = save(&[b"\x89PNG\r\n\x1a\n"]);
        assert_eq!(second.file_name().unwrap(), "pic (1).png");
        // No temporary files are left.
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn bad_transfers_are_refused_and_cleaned_up() {
        let dir = tempfile::tempdir().unwrap();
        let r = Receiver::default();
        let id = r.start(dir.path(), "x", None).unwrap();
        assert!(r.piece(id, 1, "AAAA").is_err(), "pieces must come in order");
        assert!(
            r.piece(id, 0, "AAAA").is_err(),
            "the transfer is gone after an error"
        );
        let id = r.start(dir.path(), "x", None).unwrap();
        assert!(r.piece(id, 0, "!!!not base64").is_err());
        let id = r.start(dir.path(), "x", None).unwrap();
        assert!(r.finish(id).is_err(), "an empty file is not saved");
        assert!(r.finish(999).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        // Only a few at once.
        let ids: Vec<_> = (0..4)
            .map(|_| r.start(dir.path(), "x", None).unwrap())
            .collect();
        assert!(r.start(dir.path(), "x", None).is_err());
        for id in ids {
            r.abort(id);
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
