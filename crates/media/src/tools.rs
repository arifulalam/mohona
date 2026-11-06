//! The two outside programs Mohona can use, what each is for, and where to get it.
//!
//! Neither is part of Mohona (their licences differ and they change often): Mohona looks for them
//! and, when one is missing, says what needs it and where to download it.

use std::path::{Path, PathBuf};

/// An optional outside program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    /// The name people know it by.
    pub name: &'static str,
    /// The program's file name without `.exe`.
    pub program: &'static str,
    /// The environment variable that names it.
    pub variable: &'static str,
    /// What Mohona uses it for (finishes "It is needed to ...").
    pub purpose: &'static str,
    /// The official download page.
    pub page: &'static str,
    /// The page of a Windows build (for ffmpeg the official page only lists third-party builds).
    pub windows: &'static str,
    /// The command that installs it on Windows.
    pub winget: &'static str,
}

pub const FFMPEG: Tool = Tool {
    name: "ffmpeg",
    program: "ffmpeg",
    variable: "MOHONA_FFMPEG",
    purpose: "join separate video and audio, add subtitles to a video, convert files, and merge the best video and sound of a YouTube download",
    page: "https://ffmpeg.org/download.html",
    windows: "https://www.gyan.dev/ffmpeg/builds/",
    winget: "winget install Gyan.FFmpeg",
};

pub const YTDLP: Tool = Tool {
    name: "yt-dlp",
    program: "yt-dlp",
    variable: "MOHONA_YTDLP",
    purpose: "download videos from YouTube, Vimeo and similar sites (they hide the real file behind a web page that needs a program to read it)",
    page: "https://github.com/yt-dlp/yt-dlp#installation",
    windows: "https://github.com/yt-dlp/yt-dlp/releases/latest",
    winget: "winget install yt-dlp.yt-dlp",
};

/// Every address the app may open for the person (nothing else is ever opened from a message).
pub const TOOL_PAGES: &[&str] = &[
    "https://ffmpeg.org/download.html",
    "https://www.gyan.dev/ffmpeg/builds/",
    "https://github.com/yt-dlp/yt-dlp#installation",
    "https://github.com/yt-dlp/yt-dlp/releases/latest",
];

impl Tool {
    /// The words shown when the program is nowhere.
    pub fn missing_message(&self) -> String {
        format!(
            "{name} was not found. It is needed to {purpose}.\n\
             Windows: run `{winget}` in a terminal, or download it from {windows} \
             (for yt-dlp: the file yt-dlp.exe; for ffmpeg: a zip, from which you take ffmpeg.exe from the bin folder). \
             Other systems: {page}\n\
             Then put it in Mohona's folder (next to mohona.exe), add its folder to the PATH, \
             or say where it is (Settings, Tools; or --{program} PATH; or the {variable} variable).",
            name = self.name,
            purpose = self.purpose,
            winget = self.winget,
            windows = self.windows,
            page = self.page,
            program = self.program,
            variable = self.variable,
        )
    }

    fn file_name(&self) -> String {
        if cfg!(windows) {
            format!("{}.exe", self.program)
        } else {
            self.program.to_string()
        }
    }

    fn resolve(&self, path: &Path) -> Option<PathBuf> {
        if path.is_file() {
            Some(path.to_path_buf())
        } else if path.is_dir() {
            Some(path.join(self.file_name())).filter(|p| p.is_file())
        } else {
            None
        }
    }

    /// Finds the program: the path given (a file or a folder holding it), else the environment
    /// variable, else next to the running program, else the `PATH`. An explicit choice is never
    /// second-guessed by searching elsewhere.
    pub fn find(&self, given: Option<&Path>) -> Option<PathBuf> {
        let from_env = std::env::var_os(self.variable)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        if let Some(path) = given.map(Path::to_path_buf).or(from_env) {
            return self.resolve(&path);
        }
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
        {
            let candidate = dir.join(self.file_name());
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(self.file_name()))
            .find(|candidate| candidate.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_says_why_and_where() {
        for tool in [FFMPEG, YTDLP] {
            let m = tool.missing_message();
            assert!(
                m.contains("is needed to") && m.contains(tool.windows) && m.contains(tool.winget),
                "{m}"
            );
            assert!(
                m.contains(tool.variable) && m.contains(&format!("--{}", tool.program)),
                "{m}"
            );
            assert!(TOOL_PAGES.contains(&tool.page) && TOOL_PAGES.contains(&tool.windows));
        }
    }

    #[test]
    fn an_explicit_path_is_final() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(YTDLP.file_name());
        std::fs::write(&file, "").unwrap();
        assert_eq!(YTDLP.find(Some(&file)), Some(file.clone()));
        assert_eq!(YTDLP.find(Some(dir.path())), Some(file));
        assert_eq!(YTDLP.find(Some(&dir.path().join("nope"))), None);
        assert_eq!(FFMPEG.find(Some(dir.path())), None, "a folder without it");
    }
}
