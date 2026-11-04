//! Per-site profiles: how to talk to one site.
//!
//! A profile names a site and says how downloads from it should be made: at
//! most so many connections, a speed limit for each download, and a user
//! agent, referrer or extra headers to send. Profiles are tried in order and
//! the first enabled one for a download's site is used. Whatever the caller
//! sent with the download itself always wins over the profile.

use serde::{Deserialize, Serialize};

pub const SETTING: &str = "profiles.list";
const MAX_PROFILES: usize = 100;
const MAX_HEADERS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Given by the daemon when the list is saved.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The site: `example.com` (also its subdomains), `*.example.com` or a pattern with `*`.
    pub host: String,
    /// Most connections one download from this site may use (1 to 32).
    #[serde(default)]
    pub connections: Option<u32>,
    /// Bytes per second for each download from this site.
    #[serde(default)]
    pub speed_limit: Option<u64>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub referrer: Option<String>,
    /// Extra headers, as (name, value). Never cookies or logins.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

fn yes() -> bool {
    true
}

/// The first enabled profile for this address.
pub fn find<'a>(profiles: &'a [Profile], url: &str) -> Option<&'a Profile> {
    let host = crate::rules::host_of(url)?;
    profiles
        .iter()
        .find(|p| p.enabled && crate::rules::host_matches(&p.host, &host))
}

/// Headers a profile may not set: the ones that would break the request or hold secrets.
const FORBIDDEN: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "trailer",
    "expect",
    "range",
    "if-range",
    "cookie",
    "authorization",
    "proxy-authorization",
    "user-agent",
    "referer",
];

/// Whether a header may be set by something other than the person's own profile.
pub(crate) fn header_ok(name: &str, value: &str) -> bool {
    !name.is_empty()
        && name.len() <= 60
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && value.len() <= 500
        && !value.chars().any(char::is_control)
        && !FORBIDDEN.contains(&name.to_ascii_lowercase().as_str())
}

fn clean(text: &mut Option<String>, at: &str, what: &str, max: usize) -> Result<(), String> {
    if let Some(t) = text {
        *t = t.trim().to_string();
        if t.is_empty() {
            *text = None;
        } else if t.chars().count() > max || t.chars().any(char::is_control) {
            return Err(format!(
                "{at}: {what} is too long or has control characters"
            ));
        }
    }
    Ok(())
}

/// Checks a list before it is saved and numbers new profiles.
pub fn check(mut profiles: Vec<Profile>, old: &[Profile]) -> Result<Vec<Profile>, String> {
    if profiles.len() > MAX_PROFILES {
        return Err(format!("at most {MAX_PROFILES} profiles"));
    }
    let mut next = old.iter().map(|p| p.id).max().unwrap_or(0) + 1;
    let mut seen = std::collections::HashSet::new();
    for (n, p) in profiles.iter_mut().enumerate() {
        let at = format!("profile {}", n + 1);
        p.name = p.name.trim().to_string();
        if p.name.is_empty() || p.name.chars().count() > 60 || p.name.chars().any(char::is_control)
        {
            return Err(format!("{at}: the name must be 1 to 60 characters"));
        }
        p.host = p.host.trim().to_ascii_lowercase();
        if p.host.is_empty()
            || p.host.len() > 200
            || p.host == "*"
            || !p
                .host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'*' | b'_'))
        {
            return Err(format!(
                "{at}: the site is a name such as example.com (or *.example.com), not an address"
            ));
        }
        if p.connections.is_some_and(|c| !(1..=32).contains(&c)) {
            return Err(format!("{at}: connections must be from 1 to 32"));
        }
        if p.speed_limit == Some(0) {
            p.speed_limit = None;
        }
        clean(&mut p.user_agent, &at, "the user agent", 300)?;
        clean(&mut p.referrer, &at, "the referrer", 500)?;
        if p.headers.len() > MAX_HEADERS {
            return Err(format!("{at}: at most {MAX_HEADERS} headers"));
        }
        for (name, value) in &mut p.headers {
            *name = name.trim().to_string();
            *value = value.trim().to_string();
            let token = !name.is_empty()
                && name.len() <= 60
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
            if !token || value.len() > 500 || value.chars().any(char::is_control) {
                return Err(format!("{at}: the header \"{name}\" is not valid"));
            }
            if FORBIDDEN.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(format!(
                    "{at}: the header \"{name}\" cannot be set here (use the user agent and referrer fields; cookies and logins are not kept in profiles)"
                ));
            }
        }
        if p.connections.is_none()
            && p.speed_limit.is_none()
            && p.user_agent.is_none()
            && p.referrer.is_none()
            && p.headers.is_empty()
        {
            return Err(format!("{at}: give it something to set"));
        }
        if p.id <= 0 || !old.iter().any(|o| o.id == p.id) || !seen.insert(p.id) {
            p.id = next;
            next += 1;
            seen.insert(p.id);
        }
    }
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(host: &str) -> Profile {
        Profile {
            id: 0,
            name: "P".into(),
            enabled: true,
            host: host.into(),
            connections: Some(2),
            speed_limit: None,
            user_agent: None,
            referrer: None,
            headers: vec![],
        }
    }

    #[test]
    fn the_first_enabled_profile_for_the_site_is_found() {
        let mut off = profile("example.com");
        off.enabled = false;
        let mut two = profile("*.example.com");
        two.connections = Some(4);
        let list = vec![off, two.clone(), profile("example.com")];
        assert_eq!(find(&list, "https://cdn.example.com/a.zip"), Some(&two));
        assert_eq!(
            find(&list, "https://example.com/a.zip").map(|p| p.connections),
            Some(Some(2))
        );
        assert_eq!(find(&list, "https://other.test/a.zip"), None);
    }

    #[test]
    fn a_list_is_checked_and_numbered() {
        let saved = check(vec![profile("Example.COM"), profile("b.test")], &[]).unwrap();
        assert_eq!(
            (saved[0].id, saved[1].id, saved[0].host.as_str()),
            (1, 2, "example.com")
        );
        let again = check(vec![profile("c.test"), saved[1].clone()], &saved).unwrap();
        assert_eq!((again[0].id, again[1].id), (3, 2));
        for bad_host in ["", "*", "https://a.test/x", "a.test/x", "a b"] {
            assert!(check(vec![profile(bad_host)], &[]).is_err(), "{bad_host}");
        }
        let mut many = profile("a.test");
        many.connections = Some(33);
        assert!(check(vec![many], &[]).is_err());
        let mut nothing = profile("a.test");
        nothing.connections = None;
        assert!(check(vec![nothing.clone()], &[])
            .unwrap_err()
            .contains("something to set"));
        for name in ["Cookie", "authorization", "Host", "user-agent", "Range"] {
            let mut p = nothing.clone();
            p.headers = vec![(name.into(), "x".into())];
            assert!(check(vec![p], &[]).is_err(), "{name}");
        }
        for (name, value) in [("Bad Name", "x"), ("X-A", "line\nbreak")] {
            let mut p = nothing.clone();
            p.headers = vec![(name.into(), value.into())];
            assert!(check(vec![p], &[]).is_err(), "{name}");
        }
        let mut ok = nothing;
        ok.headers = vec![("X-Token-Free".into(), "1".into())];
        ok.speed_limit = Some(0);
        let saved = check(vec![ok], &[]).unwrap();
        assert_eq!(saved[0].speed_limit, None);
    }
}
