//! Simple conversions with ffmpeg: presets for the things people actually do
//! with a downloaded video (get the sound, make it smaller, make it play
//! everywhere).

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Sound only, MP3 (variable quality, about 190 kbit/s).
    Mp3,
    /// Sound only, AAC in an M4A file (copied when the sound already is AAC).
    M4a,
    /// Video that plays everywhere: H.264 + AAC in MP4.
    Mp4,
    /// A smaller MP4: at most 720 lines tall, stronger compression.
    Mp4Small,
}

pub const PRESETS: [Preset; 4] = [Preset::Mp3, Preset::M4a, Preset::Mp4, Preset::Mp4Small];

impl Preset {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "mp3" => Ok(Preset::Mp3),
            "m4a" => Ok(Preset::M4a),
            "mp4" => Ok(Preset::Mp4),
            "mp4-small" | "small" => Ok(Preset::Mp4Small),
            other => Err(format!(
                "\"{other}\" is not a preset (use mp3, m4a, mp4 or mp4-small)"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::Mp3 => "mp3",
            Preset::M4a => "m4a",
            Preset::Mp4 => "mp4",
            Preset::Mp4Small => "mp4-small",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Preset::Mp3 => "mp3",
            Preset::M4a => "m4a",
            Preset::Mp4 | Preset::Mp4Small => "mp4",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Preset::Mp3 => "sound only, MP3",
            Preset::M4a => "sound only, AAC in M4A",
            Preset::Mp4 => "video that plays everywhere: H.264 + AAC in MP4",
            Preset::Mp4Small => "smaller MP4: at most 720 lines tall, stronger compression",
        }
    }
}

/// The ffmpeg arguments for converting `input` into `output`.
pub fn arguments(input: &Path, output: &Path, preset: Preset) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]
        .iter()
        .map(Into::into)
        .collect();
    args.push(input.into());
    let rest: &[&str] = match preset {
        Preset::Mp3 => &["-vn", "-map", "0:a:0", "-c:a", "libmp3lame", "-q:a", "2"],
        Preset::M4a => &["-vn", "-map", "0:a:0", "-c:a", "aac", "-b:a", "192k"],
        Preset::Mp4 => &[
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "160k",
            "-movflags",
            "+faststart",
        ],
        Preset::Mp4Small => &[
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-vf",
            "scale=-2:'min(720,ih)'",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "28",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-movflags",
            "+faststart",
        ],
    };
    args.extend(rest.iter().map(Into::into));
    args.push(output.into());
    args
}

/// Converts `input` into `output` (written under a temporary name first, so
/// a failed conversion leaves nothing behind). Errors carry ffmpeg's words.
pub fn run(ffmpeg: &Path, input: &Path, output: &Path, preset: Preset) -> Result<(), String> {
    let stem = output.file_stem().unwrap_or_default().to_string_lossy();
    let temporary = output.with_file_name(format!("{stem}.converting.{}", preset.extension()));
    let result = Command::new(ffmpeg)
        .args(arguments(input, &temporary, preset))
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
            "ffmpeg could not convert the file: {}",
            tail.join(" | ")
        ));
    }
    std::fs::rename(&temporary, output)
        .map_err(|e| format!("cannot save {}: {e}", output.display()))
}
