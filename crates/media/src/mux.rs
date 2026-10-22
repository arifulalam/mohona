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
