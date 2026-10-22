//! Joining separate video and audio files into one, with ffmpeg.
//!
//! ffmpeg is run as a separate program, never linked, and only ever copies
//! the streams (`-c copy`): nothing is re-encoded, so it is fast and loses no
//! quality.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The file type to join into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mkv,
}

impl Container {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "mp4" => Ok(Container::Mp4),
            "mkv" => Ok(Container::Mkv),
            other => Err(format!("\"{other}\" is not a container (use mp4 or mkv)")),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
        }
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Finds ffmpeg: the path given (a file, or a folder holding it), else the
/// `MOHONA_FFMPEG` environment variable, else next to the Mohona program,
/// else the `PATH`. `None` when it is nowhere.
pub fn find_ffmpeg(given: Option<&Path>) -> Option<PathBuf> {
    let from_env = std::env::var_os("MOHONA_FFMPEG").map(PathBuf::from);
    if let Some(path) = given.map(Path::to_path_buf).or(from_env) {
        // An explicit choice is never second-guessed by searching elsewhere.
        return resolve(&path);
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        let candidate = dir.join(exe_name());
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe_name()))
        .find(|candidate| candidate.is_file())
}

fn resolve(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        Some(path.to_path_buf())
    } else if path.is_dir() {
        let inside = path.join(exe_name());
        inside.is_file().then_some(inside)
    } else {
        None
    }
}

/// The arguments for joining `video` and `audio` into `output`.
pub fn join_arguments(
    video: &Path,
    audio: &Path,
    output: &Path,
    container: Container,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> =
        ["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]
            .iter()
            .map(Into::into)
            .collect();
    args.push(video.into());
    args.push("-i".into());
    args.push(audio.into());
    for a in ["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"] {
        args.push(a.into());
    }
    if container == Container::Mp4 {
        // Puts the index first, so the file plays before it is fully loaded.
        args.push("-movflags".into());
        args.push("+faststart".into());
    }
    args.push(output.into());
    args
}

/// Joins a video file and an audio file into `output`, copying the streams.
/// On failure nothing is left at `output`; the error carries ffmpeg's words.
pub fn join(
    ffmpeg: &Path,
    video: &Path,
    audio: &Path,
    output: &Path,
    container: Container,
) -> Result<(), String> {
    // ffmpeg picks the format from the extension, so write under a name that
    // keeps it, and rename when it worked.
    let stem = output.file_stem().unwrap_or_default().to_string_lossy();
    let temporary = output.with_file_name(format!("{stem}.joining.{}", container.extension()));
    let result = Command::new(ffmpeg)
        .args(join_arguments(video, audio, &temporary, container))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run ffmpeg ({}): {e}", ffmpeg.display()))?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&temporary);
        let said = String::from_utf8_lossy(&result.stderr);
        let said = said.trim();
        let tail: Vec<&str> = said
            .lines()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!(
            "ffmpeg could not join the files: {}",
            tail.join(" | ")
        ));
    }
    if output.exists() {
        std::fs::remove_file(output)
            .map_err(|e| format!("cannot replace {}: {e}", output.display()))?;
    }
    std::fs::rename(&temporary, output)
        .map_err(|e| format!("cannot save {}: {e}", output.display()))
}

/// ISO 639-2 code for ffmpeg's `language` tag: two-letter codes of common
/// languages are widened, three-letter ones pass through, others are left
/// untagged.
pub fn language_tag(code: &str) -> Option<String> {
    let c = code.trim().to_ascii_lowercase();
    let c = c.split(['-', '_']).next().unwrap_or("");
    let wide = match c {
        "en" => "eng",
        "bn" => "ben",
        "ar" => "ara",
        "de" => "deu",
        "es" => "spa",
        "fr" => "fra",
        "hi" => "hin",
        "id" => "ind",
        "it" => "ita",
        "ja" => "jpn",
        "ko" => "kor",
        "nl" => "nld",
        "pt" => "por",
        "ru" => "rus",
        "tr" => "tur",
        "ur" => "urd",
        "zh" => "zho",
        other if other.len() == 3 && other.chars().all(|ch| ch.is_ascii_alphabetic()) => other,
        _ => return None,
    };
    Some(wide.to_string())
}

/// The arguments for adding SRT files to `input` as subtitle tracks.
pub fn embed_arguments(
    input: &Path,
    subtitles: &[(PathBuf, Option<String>)],
    output: &Path,
    container: Container,
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> =
        ["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]
            .iter()
            .map(Into::into)
            .collect();
    args.push(input.into());
    for (file, _) in subtitles {
        args.push("-i".into());
        args.push(file.into());
    }
    args.push("-map".into());
    args.push("0".into());
    for i in 0..subtitles.len() {
        args.push("-map".into());
        args.push(format!("{}:0", i + 1).into());
    }
    let codec = if container == Container::Mp4 {
        "mov_text"
    } else {
        "srt"
    };
    for a in ["-c", "copy", "-c:s", codec] {
        args.push(a.into());
    }
    for (i, (_, language)) in subtitles.iter().enumerate() {
        if let Some(language) = language.as_deref().and_then(language_tag) {
            args.push(format!("-metadata:s:s:{i}").into());
            args.push(format!("language={language}").into());
        }
    }
    if container == Container::Mp4 {
        args.push("-movflags".into());
        args.push("+faststart".into());
    }
    args.push(output.into());
    args
}

/// Adds subtitle files to `input` as tracks, writing `output` (which may be
/// the same path as `input`). Nothing is changed when ffmpeg fails.
pub fn embed_subtitles(
    ffmpeg: &Path,
    input: &Path,
    subtitles: &[(PathBuf, Option<String>)],
    output: &Path,
    container: Container,
) -> Result<(), String> {
    let stem = output.file_stem().unwrap_or_default().to_string_lossy();
    let temporary = output.with_file_name(format!("{stem}.embedding.{}", container.extension()));
    let result = Command::new(ffmpeg)
        .args(embed_arguments(input, subtitles, &temporary, container))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run ffmpeg ({}): {e}", ffmpeg.display()))?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&temporary);
        let said = String::from_utf8_lossy(&result.stderr);
        let tail: Vec<&str> = said
            .trim()
            .lines()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!(
            "ffmpeg could not add the subtitles: {}",
            tail.join(" | ")
        ));
    }
    if output.exists() {
        std::fs::remove_file(output)
            .map_err(|e| format!("cannot replace {}: {e}", output.display()))?;
    }
    std::fs::rename(&temporary, output)
        .map_err(|e| format!("cannot save {}: {e}", output.display()))
}
