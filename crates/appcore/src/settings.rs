//! The browser extension's settings, as the engine keeps them: one JSON text
//! that the extension and the app both edit (see ADR-0047). The app changes
//! only the fields it knows and leaves any others as they are.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtensionSettings {
    /// Take over the browser's downloads.
    pub intercept: bool,
    /// Leave files smaller than this to the browser (0: take all).
    pub min_size_bytes: u64,
    pub excluded_hosts: Vec<String>,
    /// Show the download button on videos.
    pub video_button: bool,
    pub button_hidden_hosts: Vec<String>,
    /// Renew expired links by itself.
    pub auto_refresh: bool,
}

impl Default for ExtensionSettings {
    fn default() -> Self {
        ExtensionSettings {
            intercept: true,
            min_size_bytes: 0,
            excluded_hosts: Vec::new(),
            video_button: true,
            button_hidden_hosts: Vec::new(),
            auto_refresh: true,
        }
    }
}

/// Site names the way the extension keeps them: lower case, no `*.` or dot in front.
fn clean_hosts(hosts: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in hosts {
        let h = h.trim().to_lowercase();
        let h = h
            .trim_start_matches("*.")
            .trim_start_matches('.')
            .to_string();
        if !h.is_empty()
            && !h.chars().any(|c| c.is_control() || c.is_whitespace())
            && !out.contains(&h)
        {
            out.push(h);
        }
    }
    out
}

impl ExtensionSettings {
    /// From the stored text; anything missing or wrong gets its default.
    pub fn from_text(text: Option<&str>) -> Self {
        let Some(Value::Object(map)) = text.and_then(|t| serde_json::from_str(t).ok()) else {
            return Self::default();
        };
        let d = Self::default();
        let flag = |k: &str, default: bool| map.get(k).and_then(Value::as_bool).unwrap_or(default);
        let hosts = |k: &str| -> Vec<String> {
            map.get(k)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .map(|v: Vec<String>| clean_hosts(&v))
                .unwrap_or_default()
        };
        ExtensionSettings {
            intercept: flag("intercept", d.intercept),
            min_size_bytes: map.get("minSizeBytes").and_then(Value::as_u64).unwrap_or(0),
            excluded_hosts: hosts("excludedHosts"),
            video_button: flag("videoButton", d.video_button),
            button_hidden_hosts: hosts("buttonHiddenHosts"),
            auto_refresh: flag("autoRefresh", d.auto_refresh),
        }
    }
}

/// The text to store: `current` with the app's fields replaced and a new
/// `updated` time, so the browsers see the change as newer than theirs.
pub fn merge_extension(current: Option<&str>, new: &ExtensionSettings, now_ms: u64) -> String {
    let mut map: Map<String, Value> = match current.and_then(|t| serde_json::from_str(t).ok()) {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let new = json!({
        "intercept": new.intercept,
        "minSizeBytes": new.min_size_bytes,
        "excludedHosts": clean_hosts(&new.excluded_hosts),
        "videoButton": new.video_button,
        "buttonHiddenHosts": clean_hosts(&new.button_hidden_hosts),
        "autoRefresh": new.auto_refresh,
    });
    if let Value::Object(fields) = new {
        map.extend(fields);
    }
    // Strictly newer than what was there, even when clocks disagree.
    let before = map.get("updated").and_then(Value::as_u64).unwrap_or(0);
    map.insert("updated".into(), json!(now_ms.max(before + 1)));
    Value::Object(map).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_text_gives_the_defaults() {
        for text in [
            None,
            Some(""),
            Some("not json"),
            Some("[1]"),
            Some("{\"intercept\":\"yes\",\"minSizeBytes\":-5}"),
        ] {
            assert_eq!(
                ExtensionSettings::from_text(text),
                ExtensionSettings::default(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn reads_what_the_extension_wrote() {
        let text = r#"{"intercept":false,"minSizeBytes":1048576,"excludedHosts":["Example.com","*.b.org"],"videoButton":false,"autoRefresh":false,"updated":5}"#;
        let s = ExtensionSettings::from_text(Some(text));
        assert!(!s.intercept && !s.video_button && !s.auto_refresh);
        assert_eq!(s.min_size_bytes, 1_048_576);
        assert_eq!(s.excluded_hosts, ["example.com", "b.org"]);
    }

    #[test]
    fn merging_keeps_unknown_fields_and_makes_the_copy_newer() {
        let current = r#"{"futureField":7,"intercept":true,"updated":100}"#;
        let mut s = ExtensionSettings::from_text(Some(current));
        s.intercept = false;
        s.excluded_hosts = vec![
            " Site.ORG ".into(),
            "site.org".into(),
            "".into(),
            "bad host".into(),
        ];
        let text = merge_extension(Some(current), &s, 50);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["futureField"], 7);
        assert_eq!(v["intercept"], false);
        assert_eq!(v["excludedHosts"], json!(["site.org"]));
        assert_eq!(
            v["updated"], 101,
            "newer than the old copy even with a slow clock"
        );
        assert!(merge_extension(None, &s, 9).contains("\"updated\":9"));
    }
}
