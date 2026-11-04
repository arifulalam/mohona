//! Settings that follow a person between computers through a folder they already sync
//! (OneDrive, Dropbox, Syncthing ...). No server: each computer merges with one JSON file.
//!
//! Only portable settings travel: never folders, tokens, ports or speed caps, which belong to one
//! computer. For each setting the newer change wins.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The file's name inside the synced folder.
pub const FILE_NAME: &str = "mohona-settings.json";
/// Where the daemon keeps the chosen folder.
pub const FOLDER_SETTING: &str = "sync.folder";
/// Where the daemon keeps what it saw at the last sync.
pub const STATE_SETTING: &str = "sync.state";
/// The settings that travel.
pub const KEYS: &[&str] = &[
    "rules.list",
    "profiles.list",
    "extension.settings",
    "notifications",
    "keep_in_tray",
    "sort_by_type",
];
const MAX_VALUE: usize = 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub value: String,
    /// Seconds since 1970 of the last change.
    pub updated: u64,
    pub device: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct File {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub sections: BTreeMap<String, Section>,
}

/// What this computer saw at the last sync, to tell its own changes from old ones.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub hash: String,
    pub updated: u64,
}

pub type Marks = BTreeMap<String, Mark>;

#[derive(Debug, Default)]
pub struct Outcome {
    /// Settings to save on this computer.
    pub apply: Vec<(String, String)>,
    /// The file to write, if it changed.
    pub file: Option<File>,
    pub marks: Marks,
    /// Names of the settings written to the file.
    pub wrote: Vec<String>,
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether a value from the file may be saved here.
pub fn valid(key: &str, value: &str) -> Result<(), String> {
    if !KEYS.contains(&key) {
        return Err(format!("{key} does not travel"));
    }
    if value.len() > MAX_VALUE {
        return Err("too large".into());
    }
    match key {
        "rules.list" => {
            let rules: Vec<crate::rules::Rule> =
                serde_json::from_str(value).map_err(|e| e.to_string())?;
            crate::rules::check(rules.clone(), &rules).map(|_| ())
        }
        "profiles.list" => {
            let profiles: Vec<crate::profiles::Profile> =
                serde_json::from_str(value).map_err(|e| e.to_string())?;
            crate::profiles::check(profiles.clone(), &profiles).map(|_| ())
        }
        "extension.settings" => serde_json::from_str::<serde_json::Value>(value)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        _ if value == "0" || value == "1" => Ok(()),
        _ => Err("must be 0 or 1".into()),
    }
}

/// Merges this computer's settings with the file's. Pure: the caller reads and writes.
pub fn merge(
    local: &BTreeMap<String, String>,
    marks: &Marks,
    file: Option<File>,
    now: u64,
    device: &str,
) -> Outcome {
    let mut out = Outcome::default();
    let mut file = file.unwrap_or_default();
    let mut changed_file = file.version != 1;
    file.version = 1;
    for key in KEYS {
        let have = local.get(*key);
        // When did this computer last change it?
        let local_updated = match (have, marks.get(*key)) {
            (None, _) => 0,
            (Some(v), Some(m)) if m.hash == hash(v) => m.updated,
            (Some(_), _) => now,
        };
        let theirs = file
            .sections
            .get(*key)
            .filter(|s| valid(key, &s.value).is_ok())
            .cloned();
        let winner = match (have, theirs) {
            (None, None) => None,
            (Some(v), None) => Some((v.clone(), local_updated, true)),
            (None, Some(s)) => Some((s.value, s.updated, false)),
            (Some(v), Some(s)) if *v == s.value => {
                Some((s.value, s.updated.max(local_updated), false))
            }
            (Some(v), Some(s)) => {
                // The newer change wins; at the same second the larger hash does, so both
                // computers pick the same one.
                let local_wins = local_updated > s.updated
                    || (local_updated == s.updated && hash(v) > hash(&s.value));
                if local_wins {
                    Some((v.clone(), local_updated, true))
                } else {
                    Some((s.value, s.updated, false))
                }
            }
        };
        let Some((value, updated, local_side)) = winner else {
            continue;
        };
        if local_side {
            let same = file
                .sections
                .get(*key)
                .is_some_and(|s| s.value == value && s.updated == updated);
            if !same {
                file.sections.insert(
                    (*key).to_string(),
                    Section {
                        value: value.clone(),
                        updated,
                        device: device.to_string(),
                    },
                );
                out.wrote.push((*key).to_string());
                changed_file = true;
            }
        } else if have != Some(&value) {
            out.apply.push(((*key).to_string(), value.clone()));
        }
        out.marks.insert(
            (*key).to_string(),
            Mark {
                hash: hash(&value),
                updated,
            },
        );
    }
    if changed_file {
        out.file = Some(file);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
    fn file(pairs: &[(&str, &str, u64)]) -> File {
        File {
            version: 1,
            sections: pairs
                .iter()
                .map(|(k, v, u)| {
                    (
                        k.to_string(),
                        Section {
                            value: v.to_string(),
                            updated: *u,
                            device: "other".into(),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn a_first_sync_writes_what_this_computer_has() {
        let out = merge(
            &map(&[("notifications", "1")]),
            &Marks::new(),
            None,
            100,
            "pc",
        );
        assert_eq!(out.wrote, vec!["notifications"]);
        assert!(out.apply.is_empty());
        let f = out.file.unwrap();
        assert_eq!(f.sections["notifications"].updated, 100);
        assert_eq!(out.marks["notifications"].updated, 100);
    }

    #[test]
    fn a_new_computer_takes_what_the_file_has() {
        let f = file(&[("keep_in_tray", "1", 50)]);
        let out = merge(&map(&[]), &Marks::new(), Some(f), 100, "laptop");
        assert_eq!(
            out.apply,
            vec![("keep_in_tray".to_string(), "1".to_string())]
        );
        assert!(out.file.is_none());
        assert_eq!(out.marks["keep_in_tray"].updated, 50);
    }

    #[test]
    fn the_newer_change_wins_on_each_side() {
        let h = |v: &str| Mark {
            hash: hash(v),
            updated: 10,
        };
        let marks: Marks = [("sort_by_type".to_string(), h("0"))].into();
        // Unchanged here, newer in the file: take the file's.
        let out = merge(
            &map(&[("sort_by_type", "0")]),
            &marks,
            Some(file(&[("sort_by_type", "1", 90)])),
            100,
            "pc",
        );
        assert_eq!(
            out.apply,
            vec![("sort_by_type".to_string(), "1".to_string())]
        );
        // Changed here since the last sync (so now), older in the file: write ours.
        let out = merge(
            &map(&[("sort_by_type", "1")]),
            &marks,
            Some(file(&[("sort_by_type", "0", 90)])),
            100,
            "pc",
        );
        assert!(out.apply.is_empty());
        assert_eq!(out.file.unwrap().sections["sort_by_type"].value, "1");
        assert_eq!(out.marks["sort_by_type"].updated, 100);
    }

    #[test]
    fn two_computers_settle_on_the_same_value_when_the_times_tie() {
        let a = merge(
            &map(&[("notifications", "0")]),
            &Marks::new(),
            Some(file(&[("notifications", "1", 100)])),
            100,
            "a",
        );
        let b = merge(
            &map(&[("notifications", "1")]),
            &Marks::new(),
            Some(file(&[("notifications", "0", 100)])),
            100,
            "b",
        );
        let after_a = a.apply.first().map_or("0", |(_, v)| v.as_str()).to_string();
        let after_b = b.apply.first().map_or("1", |(_, v)| v.as_str()).to_string();
        assert_eq!(after_a, after_b);
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let first = merge(
            &map(&[("notifications", "1")]),
            &Marks::new(),
            None,
            100,
            "pc",
        );
        let again = merge(
            &map(&[("notifications", "1")]),
            &first.marks,
            first.file,
            500,
            "pc",
        );
        assert!(again.apply.is_empty() && again.file.is_none() && again.wrote.is_empty());
        assert_eq!(again.marks["notifications"].updated, 100);
    }

    #[test]
    fn bad_values_and_unknown_settings_from_the_file_are_ignored() {
        let f = file(&[
            ("notifications", "maybe", 999),
            ("api.tokens", "[]", 999),
            ("rules.list", "{", 999),
        ]);
        let out = merge(
            &map(&[("notifications", "1")]),
            &Marks::new(),
            Some(f),
            100,
            "pc",
        );
        assert!(out.apply.is_empty());
        assert_eq!(out.file.unwrap().sections["notifications"].value, "1");
        assert!(valid("api.tokens", "[]").is_err());
        assert!(valid("rules.list", "[]").is_ok());
        assert!(valid("extension.settings", "not json").is_err());
    }
}
