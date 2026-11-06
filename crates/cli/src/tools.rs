//! `mohona preview` and `mohona convert`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use mohona_core::{previewable_part, sanitize_filename, unique_path};
use mohona_media::convert::{self, Preset, PRESETS};
use mohona_media::mux;

use crate::format_bytes;

/// Opens a file in the program the system uses for it.
fn open_with_system(path: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {}: {e}", path.display()))
}

/// Copies the first `length` bytes of `from` to `to`.
fn copy_prefix(from: &Path, to: &Path, length: u64) -> std::io::Result<()> {
    let mut input = std::fs::File::open(from)?.take(length);
    let mut output = std::fs::File::create(to)?;
    std::io::copy(&mut input, &mut output)?;
    output.flush()
}

pub(crate) fn preview(target: &str, folder: &Path, no_open: bool) -> Result<(), String> {
    let as_file = Path::new(target);
    let file = if as_file.is_file() {
        as_file.to_path_buf()
    } else {
        let Some((part, ready, name)) = previewable_part(folder, target) else {
            return Err(format!(
                "nothing to preview: {target} is not a file, and no unfinished download of it was found in {}\n\
                 (give the same address and -o folder as the download, or a file name)",
                folder.display()
            ));
        };
        if ready == 0 {
            return Err(
                "nothing is complete yet at the start of the file; try again in a moment".into(),
            );
        }
        // The `.part` name hides the file type from the player, and the end
        // of the file is still empty, so a copy of the complete start is used.
        let dir = std::env::temp_dir().join("mohona-preview");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let copy = dir.join(sanitize_filename(&name));
        copy_prefix(&part, &copy, ready).map_err(|e| format!("cannot prepare the preview: {e}"))?;
        println!(
            "Preview of the first {} of the download (it is not finished: playback stops there).",
            format_bytes(ready)
        );
        copy
    };
    println!("{}", file.display());
    if no_open {
        return Ok(());
    }
    open_with_system(&file)
}

/// The file name a conversion result gets.
fn converted_name(input: &Path, preset: Preset) -> String {
    let stem = input
        .file_stem()
        .map_or_else(|| "file".to_string(), |s| s.to_string_lossy().into_owned());
    match preset {
        Preset::Mp3 | Preset::M4a => format!("{stem}.{}", preset.extension()),
        Preset::Mp4 => format!("{stem}.converted.mp4"),
        Preset::Mp4Small => format!("{stem}.small.mp4"),
    }
}

pub(crate) fn list_presets() {
    println!("Presets for `mohona convert --preset NAME`:");
    for p in PRESETS {
        println!("  {:<10} {}", p.name(), p.describe());
    }
}

pub(crate) fn convert(
    file: &Path,
    preset: &str,
    output: Option<&Path>,
    ffmpeg: Option<&Path>,
) -> Result<(), String> {
    let preset = Preset::parse(preset)?;
    if !file.is_file() {
        return Err(format!("{} is not a file", file.display()));
    }
    let Some(ffmpeg) = mux::find_ffmpeg(ffmpeg) else {
        return Err(mohona_media::tools::FFMPEG.missing_message());
    };
    let folder: PathBuf = match output {
        Some(folder) => folder.to_path_buf(),
        None => file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
    };
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("cannot use the folder {}: {e}", folder.display()))?;
    let destination = unique_path(&folder, &converted_name(file, preset));
    println!("Converting {} ({}) ...", file.display(), preset.describe());
    convert::run(&ffmpeg, file, &destination, preset)?;
    let size = std::fs::metadata(&destination).map_or(0, |m| m.len());
    println!("Saved {} ({})", destination.display(), format_bytes(size));
    Ok(())
}
