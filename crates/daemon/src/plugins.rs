//! Plugins: small programs a person installs to teach Mohona about one site.
//!
//! A plugin is a folder in the `plugins` folder next to the daemon's files, with a `plugin.json`:
//! the sites it handles and a command. When a link to one of those sites is added, Mohona runs the
//! command with the link as its last argument and reads one JSON object from its output:
//! `{"url": "https://...", "referrer": "...", "user_agent": "...", "headers": {"Name": "value"}}`.
//! The address in `url` is what gets downloaded. Plugins are off until the person switches them on.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

pub const ENABLED_SETTING: &str = "plugins.enabled";
const MAX_OUTPUT: u64 = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct Manifest {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    hosts: Vec<String>,
    command: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub description: String,
    pub hosts: Vec<String>,
    pub command: Vec<String>,
    pub dir: PathBuf,
}

/// What the person is shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub hosts: Vec<String>,
    pub enabled: bool,
    /// Why the folder is not a usable plugin, if it is not.
    pub problem: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub url: String,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Vec<(String, String)>,
}

fn load(dir: &Path) -> Result<Plugin, String> {
    let file = dir.join("plugin.json");
    let meta = std::fs::metadata(&file).map_err(|_| "no plugin.json".to_string())?;
    if meta.len() > 16 * 1024 {
        return Err("plugin.json is too large".into());
    }
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("plugin.json: {e}"))?;
    let name = m.name.trim().to_string();
    if name.is_empty()
        || name.chars().count() > 40
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'))
    {
        return Err("name: 1 to 40 letters, digits, spaces, _ - .".into());
    }
    if m.hosts.is_empty()
        || m.hosts.len() > 20
        || m.hosts.iter().any(|h| h.trim().is_empty() || h.len() > 100)
    {
        return Err("hosts: 1 to 20 sites".into());
    }
    if m.command.is_empty()
        || m.command.len() > 8
        || m.command.iter().any(|c| c.is_empty() || c.len() > 300)
    {
        return Err("command: 1 to 8 words".into());
    }
    let program = &m.command[0];
    // A program found on the PATH ("python"), or a file inside the plugin's folder: nothing else.
    let has_separator = program.contains('/') || program.contains('\\');
    if has_separator
        && (Path::new(program).is_absolute()
            || program.starts_with(['/', '\\'])
            || program.contains(':')
            || program.split(['/', '\\']).any(|p| p == ".."))
    {
        return Err(
            "command: the program must be a name on the PATH or a path inside the plugin's folder"
                .into(),
        );
    }
    Ok(Plugin {
        name,
        version: m.version.chars().take(20).collect(),
        description: m.description.chars().take(200).collect(),
        hosts: m.hosts.iter().map(|h| h.trim().to_string()).collect(),
        command: m.command,
        dir: dir.to_path_buf(),
    })
}

/// Every folder in `root`, usable or not (sorted by folder name).
pub fn discover(root: &Path) -> Vec<(String, Result<Plugin, String>)> {
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map(|r| {
            r.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs.into_iter()
        .take(100)
        .map(|d| {
            let label = d
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            (label, load(&d))
        })
        .collect()
}

/// The enabled plugin that handles `url`, if any.
pub fn find<'a>(plugins: &'a [Plugin], enabled: &[String], url: &str) -> Option<&'a Plugin> {
    let host = crate::rules::host_of(url)?;
    plugins.iter().find(|p| {
        enabled.contains(&p.name) && p.hosts.iter().any(|h| crate::rules::host_matches(h, &host))
    })
}

fn string_of(v: &serde_json::Value, key: &str, max: usize) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s))
            if s.len() <= max && !s.chars().any(char::is_control) =>
        {
            Ok(Some(s.clone()))
        }
        _ => Err(format!("\"{key}\" is not valid")),
    }
}

/// Reads what a plugin printed.
pub fn parse(output: &str) -> Result<Resolved, String> {
    let v: serde_json::Value = serde_json::from_str(output.trim())
        .map_err(|_| "it did not print one JSON object".to_string())?;
    let url = string_of(&v, "url", 4096)?.ok_or("it gave no \"url\"")?;
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || crate::rules::host_of(&url).is_none()
    {
        return Err("\"url\" must be an http or https address".into());
    }
    let mut headers = Vec::new();
    if let Some(h) = v.get("headers") {
        let map = h.as_object().ok_or("\"headers\" must be an object")?;
        if map.len() > 10 {
            return Err("at most 10 headers".into());
        }
        for (name, value) in map {
            let value = value.as_str().unwrap_or("");
            if !crate::profiles::header_ok(name, value) {
                return Err(format!("the header \"{name}\" is not allowed"));
            }
            headers.push((name.clone(), value.to_string()));
        }
    }
    Ok(Resolved {
        url,
        referrer: string_of(&v, "referrer", 2000)?,
        user_agent: string_of(&v, "user_agent", 300)?,
        headers,
    })
}

/// Runs the plugin for `url`.
pub async fn resolve(plugin: &Plugin, url: &str) -> Result<Resolved, String> {
    let program = &plugin.command[0];
    let program = if program.contains('/') || program.contains('\\') {
        plugin.dir.join(program)
    } else {
        PathBuf::from(program)
    };
    let mut command = tokio::process::Command::new(program);
    command
        .args(&plugin.command[1..])
        .arg(url)
        .current_dir(&plugin.dir)
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    for key in [
        "PATH",
        "SYSTEMROOT",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "LANG",
    ] {
        if let Some(v) = std::env::var_os(key) {
            command.env(key, v);
        }
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // no console window
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", plugin.command[0]))?;
    let mut out = String::new();
    let mut stdout = child.stdout.take().ok_or("no output")?.take(MAX_OUTPUT + 1);
    let run = async {
        let _ = stdout.read_to_string(&mut out).await;
        child.wait().await
    };
    let status = tokio::time::timeout(TIMEOUT, run)
        .await
        .map_err(|_| "it took longer than 30 seconds".to_string())?
        .map_err(|e| e.to_string())?;
    if out.len() as u64 > MAX_OUTPUT {
        return Err("its answer is too long".into());
    }
    if !status.success() {
        return Err(format!("it stopped with an error ({status})"));
    }
    parse(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_checked() {
        let ok = parse(r#"{"url":"https://cdn.example.com/a.zip","referrer":"https://example.com/","headers":{"X-Token":"abc"}}"#).unwrap();
        assert_eq!(ok.url, "https://cdn.example.com/a.zip");
        assert_eq!(ok.headers, vec![("X-Token".to_string(), "abc".to_string())]);
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"url":"file:///etc/passwd"}"#).is_err());
        assert!(parse(r#"{"url":"ftp://x/y"}"#).is_err());
        assert!(parse(r#"{"nope":1}"#).is_err());
        assert!(parse(r#"{"url":"https://a.example/b","headers":{"Cookie":"x=1"}}"#).is_err());
        assert!(parse(r#"{"url":"https://a.example/b","headers":{"Bad Name":"x"}}"#).is_err());
    }

    #[test]
    fn folders_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str, json: &str| {
            let d = dir.path().join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("plugin.json"), json).unwrap();
        };
        make(
            "good",
            r#"{"name":"Good","hosts":["example.com"],"command":["python","run.py"]}"#,
        );
        make(
            "inside",
            r#"{"name":"Inside","hosts":["a.com"],"command":["bin/run.sh"]}"#,
        );
        make(
            "abs",
            r#"{"name":"Abs","hosts":["a.com"],"command":["/bin/sh"]}"#,
        );
        make(
            "up",
            r#"{"name":"Up","hosts":["a.com"],"command":["../x"]}"#,
        );
        make(
            "drive",
            r#"{"name":"Drive","hosts":["a.com"],"command":["C:\\x.exe"]}"#,
        );
        make("nohosts", r#"{"name":"None","hosts":[],"command":["x"]}"#);
        make("broken", "{");
        std::fs::create_dir_all(dir.path().join("empty")).unwrap();
        let found = discover(dir.path());
        let ok: Vec<_> = found
            .iter()
            .filter(|(_, r)| r.is_ok())
            .map(|(l, _)| l.as_str())
            .collect();
        assert_eq!(ok, vec!["good", "inside"]);
        assert_eq!(found.len(), 8);
        let plugins: Vec<Plugin> = found.into_iter().filter_map(|(_, r)| r.ok()).collect();
        let on = vec!["Good".to_string()];
        assert!(find(&plugins, &on, "https://sub.example.com/x").is_some());
        assert!(find(&plugins, &on, "https://example.com.evil.net/x").is_none());
        assert!(
            find(&plugins, &[], "https://example.com/x").is_none(),
            "off until switched on"
        );
    }
}
