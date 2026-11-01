//! The rules engine: "when a download matches this, do that".
//!
//! A rule has conditions (all given ones must match) and actions. Rules are
//! tried in order and the first enabled rule that matches wins. They apply
//! to every way of adding a download (extension, app, API, command line), but
//! never override what the caller asked for explicitly (a folder or queue).

use serde::{Deserialize, Serialize};

pub const SETTING: &str = "rules.list";
const MAX_RULES: usize = 100;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct When {
    /// The site: `example.com` (also its subdomains), `*.example.com` (subdomains only) or any pattern with `*`.
    #[serde(default)]
    pub host: Option<String>,
    /// File types without dots, such as `["zip", "iso"]`.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Text that must be in the address (any case).
    #[serde(default)]
    pub url_contains: Option<String>,
    /// The site of the page the link was on (same patterns as `host`).
    #[serde(default)]
    pub referrer_host: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Then {
    /// Save in this folder: a full path, or a folder name inside the download folder.
    #[serde(default)]
    pub folder: Option<String>,
    /// Put it in this queue.
    #[serde(default)]
    pub queue: Option<i64>,
    /// Move the finished file into its category's folder (or not).
    #[serde(default)]
    pub sort: Option<bool>,
    /// Refuse the download.
    #[serde(default)]
    pub block: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Given by the daemon when the list is saved (1, 2, ... in order of creation).
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub when: When,
    pub then: Then,
}

fn yes() -> bool {
    true
}

/// What a download looks like to the rules.
pub struct Subject<'a> {
    pub url: &'a str,
    pub referrer: Option<&'a str>,
}

/// The host of an address, lower case, without user, port or brackets' port.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The file type in an address's path, lower case.
pub fn extension_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |p| p.1);
    let path = rest.split(['?', '#']).next()?;
    let name = path.split_once('/')?.1.rsplit('/').next()?;
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 12).then(|| ext.to_ascii_lowercase())
}

/// `*` matches any run of characters; everything else matches itself (any case).
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.contains('*') {
        glob(pattern, host)
    } else {
        let p = pattern.to_ascii_lowercase();
        host == p || host.ends_with(&format!(".{p}"))
    }
}

impl Rule {
    pub fn matches(&self, subject: &Subject<'_>) -> bool {
        let w = &self.when;
        if let Some(pattern) = &w.host {
            if !host_of(subject.url).is_some_and(|h| host_matches(pattern, &h)) {
                return false;
            }
        }
        if !w.extensions.is_empty()
            && !extension_of(subject.url).is_some_and(|e| w.extensions.contains(&e))
        {
            return false;
        }
        if let Some(text) = &w.url_contains {
            if !subject.url.to_lowercase().contains(&text.to_lowercase()) {
                return false;
            }
        }
        if let Some(pattern) = &w.referrer_host {
            if !subject
                .referrer
                .and_then(host_of)
                .is_some_and(|h| host_matches(pattern, &h))
            {
                return false;
            }
        }
        true
    }
}

/// The first enabled rule that matches.
pub fn first_match<'a>(rules: &'a [Rule], subject: &Subject<'_>) -> Option<&'a Rule> {
    rules.iter().find(|r| r.enabled && r.matches(subject))
}

/// Checks a list before it is saved and numbers new rules; returns the clean list.
pub fn check(mut rules: Vec<Rule>, old: &[Rule]) -> Result<Vec<Rule>, String> {
    if rules.len() > MAX_RULES {
        return Err(format!("at most {MAX_RULES} rules"));
    }
    let mut next = old.iter().map(|r| r.id).max().unwrap_or(0) + 1;
    let mut seen = std::collections::HashSet::new();
    for (n, rule) in rules.iter_mut().enumerate() {
        let at = format!("rule {}", n + 1);
        rule.name = rule.name.trim().to_string();
        if rule.name.is_empty()
            || rule.name.chars().count() > 60
            || rule.name.chars().any(char::is_control)
        {
            return Err(format!("{at}: the name must be 1 to 60 characters"));
        }
        let w = &mut rule.when;
        for text in [&mut w.host, &mut w.url_contains, &mut w.referrer_host] {
            if let Some(t) = text {
                *t = t.trim().to_string();
                if t.is_empty() {
                    *text = None;
                } else if t.chars().count() > 200 || t.chars().any(char::is_control) {
                    return Err(format!(
                        "{at}: a condition is too long or has control characters"
                    ));
                }
            }
        }
        w.extensions = w
            .extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        if w.extensions.len() > 50
            || w.extensions
                .iter()
                .any(|e| e.len() > 12 || !e.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return Err(format!(
                "{at}: file types are letters and digits only (such as zip)"
            ));
        }
        if w.host.is_none()
            && w.extensions.is_empty()
            && w.url_contains.is_none()
            && w.referrer_host.is_none()
        {
            return Err(format!(
                "{at}: give at least one condition, or the rule would match everything"
            ));
        }
        let t = &mut rule.then;
        if let Some(f) = &mut t.folder {
            *f = f.trim().to_string();
            if f.is_empty() {
                t.folder = None;
            }
        }
        if let Some(f) = &t.folder {
            check_folder(f).map_err(|e| format!("{at}: {e}"))?;
        }
        if t.folder.is_none() && t.queue.is_none() && t.sort.is_none() && !t.block {
            return Err(format!("{at}: give it something to do"));
        }
        if t.block && (t.folder.is_some() || t.queue.is_some() || t.sort.is_some()) {
            return Err(format!(
                "{at}: a rule that blocks cannot also do something else"
            ));
        }
        if rule.id <= 0 || !old.iter().any(|o| o.id == rule.id) || !seen.insert(rule.id) {
            rule.id = next;
            next += 1;
            seen.insert(rule.id);
        }
    }
    Ok(rules)
}

/// A full path (`C:\...`, `\\server\...` or `/...`) rather than a name inside the download folder.
pub fn is_full_path(folder: &str) -> bool {
    let b = folder.as_bytes();
    let drive = b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/');
    std::path::Path::new(folder).is_absolute() || drive || folder.starts_with(['/', '\\'])
}

/// A folder is a full path or a plain name below the download folder (no `..`).
fn check_folder(folder: &str) -> Result<(), String> {
    if folder.chars().count() > 260 || folder.chars().any(char::is_control) {
        return Err("the folder is too long or has control characters".into());
    }
    if is_full_path(folder) {
        return Ok(());
    }
    if folder.contains(':') {
        return Err(
            "a folder is a full path (C:\\...) or a name inside the download folder".into(),
        );
    }
    if folder
        .split(['/', '\\'])
        .any(|part| part == ".." || part.is_empty())
    {
        return Err("a folder name inside the download folder cannot use \"..\"".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(when: When, then: Then) -> Rule {
        Rule {
            id: 0,
            name: "r".into(),
            enabled: true,
            when,
            then,
        }
    }
    fn subject(url: &str) -> Subject<'_> {
        Subject {
            url,
            referrer: None,
        }
    }

    #[test]
    fn hosts_and_extensions_are_read_from_addresses() {
        assert_eq!(
            host_of("https://User:pw@Files.Example.com:8443/a/b.zip?x=1").as_deref(),
            Some("files.example.com")
        );
        assert_eq!(host_of("http://[::1]:80/x").as_deref(), Some("::1"));
        assert_eq!(host_of("not a url"), None);
        assert_eq!(
            extension_of("https://x.test/a/B.TAR.GZ?token=a.b").as_deref(),
            Some("gz")
        );
        assert_eq!(extension_of("https://x.test/dir/").as_deref(), None);
        assert_eq!(extension_of("https://x.test/noext").as_deref(), None);
        assert_eq!(extension_of("https://x.test").as_deref(), None);
    }

    #[test]
    fn site_patterns_match_subdomains_but_not_look_alikes() {
        assert!(host_matches("example.com", "example.com"));
        assert!(host_matches("example.com", "cdn.example.com"));
        assert!(!host_matches("example.com", "notexample.com"));
        assert!(!host_matches("example.com", "example.com.evil.test"));
        assert!(host_matches("*.example.com", "cdn.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
        assert!(host_matches("cdn*.example.*", "cdn2.example.org"));
        assert!(host_matches("EXAMPLE.com", "example.com"));
    }

    #[test]
    fn all_given_conditions_must_match() {
        let r = rule(
            When {
                host: Some("example.com".into()),
                extensions: vec!["zip".into(), "7z".into()],
                ..When::default()
            },
            Then {
                block: true,
                ..Then::default()
            },
        );
        assert!(r.matches(&subject("https://example.com/a.zip")));
        assert!(r.matches(&subject("https://dl.example.com/a.7z")));
        assert!(!r.matches(&subject("https://example.com/a.pdf")));
        assert!(!r.matches(&subject("https://other.test/a.zip")));
        let by_page = rule(
            When {
                referrer_host: Some("news.test".into()),
                ..When::default()
            },
            Then {
                sort: Some(true),
                ..Then::default()
            },
        );
        assert!(by_page.matches(&Subject {
            url: "https://x.test/a",
            referrer: Some("https://www.news.test/story")
        }));
        assert!(!by_page.matches(&subject("https://x.test/a")));
        let text = rule(
            When {
                url_contains: Some("/ISO/".into()),
                ..When::default()
            },
            Then {
                sort: Some(true),
                ..Then::default()
            },
        );
        assert!(text.matches(&subject("https://x.test/iso/a.bin")));
    }

    #[test]
    fn the_first_enabled_match_wins() {
        let mut a = rule(
            When {
                extensions: vec!["zip".into()],
                ..When::default()
            },
            Then {
                folder: Some("A".into()),
                ..Then::default()
            },
        );
        let b = rule(
            When {
                extensions: vec!["zip".into()],
                ..When::default()
            },
            Then {
                folder: Some("B".into()),
                ..Then::default()
            },
        );
        a.enabled = false;
        let list = vec![a.clone(), b.clone()];
        assert_eq!(
            first_match(&list, &subject("https://x.test/a.zip")),
            Some(&b)
        );
        assert_eq!(first_match(&list, &subject("https://x.test/a.pdf")), None);
    }

    #[test]
    fn a_list_is_checked_and_numbered() {
        let good = rule(
            When {
                extensions: vec![".ZIP".into()],
                ..When::default()
            },
            Then {
                folder: Some("Zips".into()),
                ..Then::default()
            },
        );
        let saved = check(vec![good.clone(), good.clone()], &[]).unwrap();
        assert_eq!((saved[0].id, saved[1].id), (1, 2));
        assert_eq!(saved[0].when.extensions, vec!["zip"]);
        // Old rules keep their numbers; new ones get new numbers.
        let mut again = saved.clone();
        again.insert(0, good.clone());
        let renumbered = check(again, &saved).unwrap();
        assert_eq!(
            renumbered.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![3, 1, 2]
        );

        let no_when = rule(
            When::default(),
            Then {
                block: true,
                ..Then::default()
            },
        );
        assert!(check(vec![no_when], &[])
            .unwrap_err()
            .contains("at least one condition"));
        let no_then = rule(
            When {
                host: Some("a.test".into()),
                ..When::default()
            },
            Then::default(),
        );
        assert!(check(vec![no_then], &[])
            .unwrap_err()
            .contains("something to do"));
        let mixed = rule(
            When {
                host: Some("a.test".into()),
                ..When::default()
            },
            Then {
                block: true,
                sort: Some(true),
                ..Then::default()
            },
        );
        assert!(check(vec![mixed], &[]).is_err());
        for bad in ["../up", "a/../b", "C:evil", "a//b"] {
            let r = rule(
                When {
                    host: Some("a.test".into()),
                    ..When::default()
                },
                Then {
                    folder: Some(bad.into()),
                    ..Then::default()
                },
            );
            assert!(check(vec![r], &[]).is_err(), "{bad}");
        }
        for ok in ["Zips", "Work/Zips", "D:\\Downloads\\Zips", "/home/me/zips"] {
            let r = rule(
                When {
                    host: Some("a.test".into()),
                    ..When::default()
                },
                Then {
                    folder: Some(ok.into()),
                    ..Then::default()
                },
            );
            assert!(check(vec![r], &[]).is_ok(), "{ok}");
        }
        let mut unnamed = good;
        unnamed.name = "  ".into();
        assert!(check(vec![unnamed], &[]).is_err());
    }
}
