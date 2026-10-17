//! Categories: sort finished files into folders by extension.

use rusqlite::params;

use crate::{Store, StoreError};

/// One category.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryRow {
    /// Database id.
    pub id: i64,
    /// Display name (also the default folder name).
    pub name: String,
    /// File extensions in lower case, without dots.
    pub extensions: Vec<String>,
    /// Folder for the category's files: absolute, or relative to the folder
    /// the download was asked to go to. `None` uses the category's name.
    pub folder: Option<String>,
}

/// The extension of a file name in lower case (`a.tar.GZ` gives `gz`).
pub fn extension_of(filename: &str) -> Option<String> {
    let (stem, extension) = filename.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then(|| extension.to_ascii_lowercase())
}

impl Store {
    /// All categories.
    pub fn categories(&self) -> Result<Vec<CategoryRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,name,extensions,folder FROM categories ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| {
                let extensions: String = r.get(2)?;
                Ok(CategoryRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    extensions: extensions
                        .split(',')
                        .map(|e| e.trim().to_ascii_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect(),
                    folder: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Add a category. Extensions are written without dots (`zip`, `7z`).
    pub fn add_category(
        &self,
        name: &str,
        extensions: &[String],
        folder: Option<&str>,
    ) -> Result<i64, StoreError> {
        let extensions: Vec<String> = extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        self.conn.execute(
            "INSERT INTO categories (name,extensions,folder) VALUES (?1,?2,?3)",
            params![name.trim(), extensions.join(","), folder],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Change a category's extensions and folder.
    pub fn update_category(
        &self,
        id: i64,
        extensions: &[String],
        folder: Option<&str>,
    ) -> Result<(), StoreError> {
        let extensions: Vec<String> = extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        self.conn.execute(
            "UPDATE categories SET extensions=?2, folder=?3 WHERE id=?1",
            params![id, extensions.join(","), folder],
        )?;
        Ok(())
    }

    /// Remove a category. Downloads that were in it keep their files and
    /// simply have no category any more.
    pub fn remove_category(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM categories WHERE id=?1", [id])?;
        Ok(())
    }

    /// The category a file belongs in, by its extension. When several
    /// categories list the extension, the one added last wins, so a
    /// category made by the person beats a built-in one.
    pub fn category_for(&self, filename: &str) -> Result<Option<CategoryRow>, StoreError> {
        let Some(extension) = extension_of(filename) else {
            return Ok(None);
        };
        Ok(self
            .categories()?
            .into_iter()
            .rev()
            .find(|c| c.extensions.contains(&extension)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_read_from_names() {
        assert_eq!(extension_of("a.zip").as_deref(), Some("zip"));
        assert_eq!(extension_of("a.tar.GZ").as_deref(), Some("gz"));
        assert_eq!(extension_of("noext"), None);
        assert_eq!(extension_of(".hidden"), None);
        assert_eq!(extension_of("trailing."), None);
    }

    #[test]
    fn built_in_categories_sort_common_files() {
        let s = Store::open_in_memory().unwrap();
        let name = |f: &str| s.category_for(f).unwrap().map(|c| c.name);
        assert_eq!(name("movie.MKV").as_deref(), Some("Video"));
        assert_eq!(name("setup.exe").as_deref(), Some("Programs"));
        assert_eq!(name("a.tar.gz").as_deref(), Some("Compressed"));
        assert_eq!(name("notes.pdf").as_deref(), Some("Documents"));
        assert_eq!(name("song.flac").as_deref(), Some("Music"));
        assert_eq!(name("mystery.xyz"), None);
        assert_eq!(name("README"), None);
    }

    #[test]
    fn people_can_add_change_and_remove_categories() {
        let s = Store::open_in_memory().unwrap();
        let id = s
            .add_category("Fonts", &[".TTF".into(), "otf".into()], Some("D:/fonts"))
            .unwrap();
        let fonts = s.category_for("a.ttf").unwrap().unwrap();
        assert_eq!(fonts.extensions, ["ttf", "otf"]);
        assert_eq!(fonts.folder.as_deref(), Some("D:/fonts"));
        // The newest category wins for a shared extension.
        s.update_category(id, &["zip".into()], None).unwrap();
        assert_eq!(s.category_for("a.zip").unwrap().unwrap().name, "Fonts");
        s.remove_category(id).unwrap();
        assert_eq!(s.category_for("a.zip").unwrap().unwrap().name, "Compressed");
        assert!(
            s.add_category("Compressed", &[], None).is_err(),
            "names are unique"
        );
    }
}
