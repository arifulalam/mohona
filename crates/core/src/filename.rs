use std::path::{Path, PathBuf};

/// Names Windows refuses to use for files, with or without an extension.
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

const MAX_NAME_CHARS: usize = 200;
const MAX_EXTENSION_BYTES: usize = 20;

/// Turn a name from a server into a safe file name.
///
/// Keeps only the last path component (so `../../x` becomes `x`), replaces
/// characters that are invalid on Windows, removes trailing dots and spaces,
/// avoids Windows reserved names, and limits the length. An unusable name
/// becomes `download`.
pub fn sanitize_filename(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let replaced: String = last
        .chars()
        .map(|character| {
            if character.is_control() || "<>:\"/\\|?*".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    let trimmed = replaced.trim_end_matches(['.', ' ']).trim_start();
    if trimmed.is_empty() {
        return "download".to_string();
    }

    let mut name = trimmed.to_string();
    let stem = name.split('.').next().unwrap_or("");
    if RESERVED_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        name.insert(0, '_');
    }
    shorten(name)
}

fn shorten(name: String) -> String {
    if name.chars().count() <= MAX_NAME_CHARS {
        return name;
    }
    let (stem, extension) = match name.rfind('.') {
        Some(index) if index > 0 && name.len() - index <= MAX_EXTENSION_BYTES => {
            name.split_at(index)
        }
        _ => (name.as_str(), ""),
    };
    let keep = MAX_NAME_CHARS.saturating_sub(extension.chars().count());
    let stem: String = stem.chars().take(keep).collect();
    format!("{stem}{extension}")
}

/// A path in `dir` for `filename` that does not exist yet: the name itself,
/// or `name (1).ext`, `name (2).ext`, and so on.
pub fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    unique_path_avoiding(dir, filename, &[])
}

/// Like [`unique_path`], but a name is also taken when the name with one of
/// `suffixes` added exists. Used so a new download never picks a name whose
/// `.part` file belongs to an unfinished download.
pub fn unique_path_avoiding(dir: &Path, filename: &str, suffixes: &[&str]) -> PathBuf {
    let is_free = |path: &Path| {
        !path.exists()
            && suffixes.iter().all(|suffix| {
                let mut with_suffix = path.as_os_str().to_owned();
                with_suffix.push(suffix);
                !Path::new(&with_suffix).exists()
            })
    };
    let first = dir.join(filename);
    if is_free(&first) {
        return first;
    }
    let (stem, extension) = match filename.rfind('.') {
        Some(index) if index > 0 => filename.split_at(index),
        _ => (filename, ""),
    };
    (1u32..)
        .map(|number| dir.join(format!("{stem} ({number}){extension}")))
        .find(|candidate| is_free(candidate))
        .unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::{sanitize_filename, unique_path};

    #[test]
    fn keeps_normal_names() {
        assert_eq!(sanitize_filename("movie.mp4"), "movie.mp4");
        assert_eq!(sanitize_filename("my file (1).zip"), "my file (1).zip");
        assert_eq!(sanitize_filename("€ rates.txt"), "€ rates.txt");
    }

    #[test]
    fn strips_directories_from_either_separator() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\temp\\a.zip"), "a.zip");
        assert_eq!(sanitize_filename("/absolute/name.txt"), "name.txt");
    }

    #[test]
    fn replaces_invalid_characters() {
        assert_eq!(sanitize_filename("a:b?c*d.txt"), "a_b_c_d.txt");
        assert_eq!(sanitize_filename("tab\there.txt"), "tab_here.txt");
    }

    #[test]
    fn removes_trailing_dots_and_spaces() {
        assert_eq!(sanitize_filename("name. . "), "name");
        assert_eq!(sanitize_filename("report.pdf."), "report.pdf");
    }

    #[test]
    fn unusable_names_become_download() {
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(sanitize_filename("   "), "download");
        assert_eq!(sanitize_filename("a/"), "download");
    }

    #[test]
    fn avoids_windows_reserved_names() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_filename("com1.log"), "_com1.log");
        assert_eq!(sanitize_filename("console.txt"), "console.txt");
    }

    #[test]
    fn long_names_are_shortened_but_keep_the_extension() {
        let long = format!("{}.mp4", "x".repeat(500));
        let short = sanitize_filename(&long);
        assert_eq!(short.chars().count(), 200);
        assert!(short.ends_with(".mp4"));
    }

    #[test]
    fn unique_path_avoiding_skips_names_with_a_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt.part"), b"x").unwrap();
        assert_eq!(
            super::unique_path_avoiding(dir.path(), "a.txt", &[".part"]),
            dir.path().join("a (1).txt")
        );
        // Without the suffix list the name is considered free.
        assert_eq!(unique_path(dir.path(), "a.txt"), dir.path().join("a.txt"));
    }

    #[test]
    fn unique_path_adds_a_number() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.txt"), dir.path().join("a.txt"));
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.txt"),
            dir.path().join("a (1).txt")
        );
        std::fs::write(dir.path().join("a (1).txt"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.txt"),
            dir.path().join("a (2).txt")
        );
        std::fs::write(dir.path().join("noext"), b"x").unwrap();
        assert_eq!(
            unique_path(dir.path(), "noext"),
            dir.path().join("noext (1)")
        );
    }
}
