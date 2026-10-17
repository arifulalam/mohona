//! Sorting finished files into category folders, and spotting duplicates.

use std::path::{Path, PathBuf};

use mohona_core::{sanitize_filename, unique_path};
use mohona_store::{extension_of, CategoryRow};

/// The category a file belongs in. When several list its extension the one
/// added last wins.
pub(crate) fn category_of<'a>(
    filename: &str,
    categories: &'a [CategoryRow],
) -> Option<&'a CategoryRow> {
    let extension = extension_of(filename)?;
    categories
        .iter()
        .rev()
        .find(|c| c.extensions.contains(&extension))
}

/// The folder a category's files go in, for a download that was asked to
/// go to `base`.
pub(crate) fn folder_of(category: &CategoryRow, base: &Path) -> PathBuf {
    match &category.folder {
        Some(folder) if Path::new(folder).is_absolute() => PathBuf::from(folder),
        Some(folder) => base.join(folder),
        None => base.join(&category.name),
    }
}

/// Move a finished file into its category's folder. Returns the file's
/// new place and the category, or the file where it is when there is no
/// category for it. An existing file is never overwritten: the moved one is
/// numbered.
pub(crate) fn sort_file(
    path: &Path,
    base: &Path,
    categories: &[CategoryRow],
) -> std::io::Result<(PathBuf, Option<i64>)> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Ok((path.to_path_buf(), None));
    };
    let Some(category) = category_of(name, categories) else {
        return Ok((path.to_path_buf(), None));
    };
    let folder = folder_of(category, base);
    if path.parent() == Some(folder.as_path()) {
        return Ok((path.to_path_buf(), Some(category.id)));
    }
    std::fs::create_dir_all(&folder)?;
    let target = unique_path(&folder, name);
    if std::fs::rename(path, &target).is_err() {
        // Another disk: copy, then remove the original.
        std::fs::copy(path, &target)?;
        std::fs::remove_file(path)?;
    }
    Ok((target, Some(category.id)))
}

/// The file name an address most likely gives, from its last path segment
/// (`None` when it ends in a slash or has no path). The server may name the
/// file differently, so this is only a guess for spotting duplicates early.
pub(crate) fn guess_filename(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://")?.1;
    let rest = rest.split(['?', '#']).next()?;
    let (_, path) = rest.split_once('/')?;
    let segment = path.rsplit('/').next()?;
    if segment.is_empty() {
        return None;
    }
    Some(sanitize_filename(&percent_decode(segment)))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat(id: i64, name: &str, exts: &[&str], folder: Option<&str>) -> CategoryRow {
        CategoryRow {
            id,
            name: name.into(),
            extensions: exts.iter().map(|e| e.to_string()).collect(),
            folder: folder.map(str::to_string),
        }
    }

    #[test]
    fn files_are_moved_into_the_category_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.zip");
        std::fs::write(&file, b"zip").unwrap();
        let cats = [cat(1, "Compressed", &["zip"], None)];
        let (moved, id) = sort_file(&file, dir.path(), &cats).unwrap();
        assert_eq!(moved, dir.path().join("Compressed").join("a.zip"));
        assert_eq!(id, Some(1));
        assert!(!file.exists());
        assert_eq!(std::fs::read(&moved).unwrap(), b"zip");
    }

    #[test]
    fn an_existing_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Compressed");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.zip"), b"old").unwrap();
        let file = dir.path().join("a.zip");
        std::fs::write(&file, b"new").unwrap();
        let cats = [cat(1, "Compressed", &["zip"], None)];
        let (moved, _) = sort_file(&file, dir.path(), &cats).unwrap();
        assert_eq!(moved, folder.join("a (1).zip"));
        assert_eq!(std::fs::read(folder.join("a.zip")).unwrap(), b"old");
    }

    #[test]
    fn folders_may_be_absolute_or_relative() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let abs = elsewhere.path().to_string_lossy().into_owned();
        let cats = [
            cat(1, "Zips", &["zip"], Some(&abs)),
            cat(2, "Docs", &["pdf"], Some("papers/pdf")),
        ];
        assert_eq!(folder_of(&cats[0], dir.path()), elsewhere.path());
        assert_eq!(
            folder_of(&cats[1], dir.path()),
            dir.path().join("papers/pdf")
        );
    }

    #[test]
    fn unknown_files_stay_and_known_ones_in_place_stay() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.xyz");
        std::fs::write(&file, b"x").unwrap();
        let cats = [cat(1, "Compressed", &["zip"], None)];
        assert_eq!(
            sort_file(&file, dir.path(), &cats).unwrap(),
            (file.clone(), None)
        );
        // Already where its category goes.
        let folder = dir.path().join("Compressed");
        std::fs::create_dir_all(&folder).unwrap();
        let zip = folder.join("b.zip");
        std::fs::write(&zip, b"z").unwrap();
        assert_eq!(
            sort_file(&zip, dir.path(), &cats).unwrap(),
            (zip.clone(), Some(1))
        );
    }

    #[test]
    fn the_newest_category_wins() {
        let cats = [
            cat(1, "Compressed", &["zip"], None),
            cat(2, "Mine", &["zip"], None),
        ];
        assert_eq!(category_of("a.zip", &cats).unwrap().name, "Mine");
    }

    #[test]
    fn names_are_guessed_from_addresses() {
        let guess = |u: &str| guess_filename(u);
        assert_eq!(guess("https://x.org/files/a.zip").as_deref(), Some("a.zip"));
        assert_eq!(
            guess("https://x.org/a%20b.zip?dl=1#top").as_deref(),
            Some("a b.zip")
        );
        assert_eq!(guess("https://x.org/dir/"), None);
        assert_eq!(guess("https://x.org"), None);
        assert_eq!(guess("not a url"), None);
    }
}
