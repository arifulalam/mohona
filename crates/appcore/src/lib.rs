//! What the Mohona app does with the engine, as plain functions.
//!
//! The window code (Tauri) only forwards its commands here, so everything
//! that can go wrong is tested without a window. The app is a client of the
//! daemon like the command line and the browser extension: it finds the
//! daemon through the info file, starts it when needed, and asks for the list
//! of downloads about once a second.

mod rows;
mod settings;
mod speed;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mohona_daemon::{Call, Client, ClientError, Response};
use serde::Serialize;

pub use rows::{name_of, Row};
pub use settings::{merge_extension, ExtensionSettings};
pub use speed::SpeedTracker;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Mohona is not running")]
    NotRunning,
    #[error("{0}")]
    Engine(String),
    #[error("{0}")]
    Input(String),
}

impl From<ClientError> for AppError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::NotRunning => AppError::NotRunning,
            ClientError::Protocol(m) => AppError::Engine(m),
        }
    }
}

/// Errors reach the window as plain text.
impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EngineStatus {
    pub running: bool,
    pub version: Option<String>,
    /// Where finished downloads go by default is the engine's business; the
    /// app only shows this when it knows it.
    pub message: Option<String>,
}

/// The engine's own settings, as the settings page shows them.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub download_folder: String,
    pub default_folder: String,
    /// Bytes per second; `None` for no limit.
    pub speed_limit: Option<u64>,
    pub max_active: u32,
}

/// What may be changed; what is `None` stays as it is.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConfigChange {
    pub download_folder: Option<String>,
    /// Bytes per second; 0 removes the limit.
    pub speed_limit: Option<u64>,
    pub max_active: Option<u32>,
}

/// The settings key the browser extension and the app share.
const EXTENSION_KEY: &str = "extension.settings";

/// How to reach (and start) the engine.
#[derive(Debug, Clone)]
pub struct Engine {
    pub info_path: PathBuf,
    /// The `mohona` program that runs `daemon`; `None` when it was not found.
    pub program: Option<PathBuf>,
    /// The database the started engine should use (`None`: its default).
    pub database: Option<PathBuf>,
}

/// How long to wait for a started engine to answer.
const START_WAIT: Duration = Duration::from_secs(10);

impl Engine {
    /// The default places: the info file, and the `mohona` program named by
    /// `MOHONA_PROGRAM`, or next to the app, or on the `PATH`.
    pub fn standard() -> Self {
        Engine {
            info_path: mohona_daemon::default_info_path(),
            program: find_program(
                std::env::var_os("MOHONA_PROGRAM"),
                std::env::current_exe().ok().as_deref(),
                std::env::var_os("PATH"),
            ),
            database: None,
        }
    }

    fn client(&self) -> Result<Client, AppError> {
        Client::from_info_file(&self.info_path).map_err(AppError::from)
    }

    async fn call(&self, call: Call) -> Result<Response, AppError> {
        match self.client()?.call(call).await? {
            Response::Error { message } => Err(AppError::Engine(message)),
            other => Ok(other),
        }
    }

    pub async fn status(&self) -> EngineStatus {
        match self.call(Call::Ping).await {
            Ok(Response::Pong { version, .. }) => EngineStatus {
                running: true,
                version: Some(version),
                message: None,
            },
            Err(AppError::NotRunning) => EngineStatus {
                running: false,
                version: None,
                message: None,
            },
            Err(e) => EngineStatus {
                running: false,
                version: None,
                message: Some(e.to_string()),
            },
            Ok(_) => EngineStatus {
                running: false,
                version: None,
                message: Some("unexpected answer from the engine".into()),
            },
        }
    }

    /// Starts the engine when it is not running; waits until it answers.
    pub async fn start(&self) -> Result<EngineStatus, AppError> {
        let status = self.status().await;
        if status.running {
            return Ok(status);
        }
        let program = self.program.as_ref().ok_or_else(|| {
            AppError::Engine("the mohona program was not found next to the app".into())
        })?;
        spawn(program, &self.info_path, self.database.as_deref())
            .map_err(|e| AppError::Engine(format!("could not start Mohona: {e}")))?;
        let started = Instant::now();
        while started.elapsed() < START_WAIT {
            let status = self.status().await;
            if status.running {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(AppError::Engine(
            "Mohona was started but did not answer in time".into(),
        ))
    }

    /// Every download, with speed and time left worked out by `tracker`.
    pub async fn list(&self, tracker: &mut SpeedTracker) -> Result<Vec<Row>, AppError> {
        match self.call(Call::List).await? {
            Response::Downloads { downloads } => Ok(tracker.rows(downloads, Instant::now())),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// Adds one address; returns the new download's id.
    pub async fn add(&self, url: &str, folder: Option<&str>) -> Result<i64, AppError> {
        let url = url.trim();
        if url.is_empty() || url.chars().any(char::is_control) {
            return Err(AppError::Input("type or paste an address".into()));
        }
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("ftp://"))
        {
            return Err(AppError::Input(
                "the address must start with http://, https:// or ftp://".into(),
            ));
        }
        let folder = folder.map(str::trim).filter(|f| !f.is_empty());
        match self
            .call(Call::Add {
                url: url.to_string(),
                dest_dir: folder.map(str::to_string),
                cookies: None,
                referrer: None,
                user_agent: None,
                headers: Vec::new(),
                sort: false,
                duplicates: Some("skip".into()),
            })
            .await?
        {
            Response::Added { id } => Ok(id),
            Response::Skipped { reason } => Err(AppError::Input(reason)),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    pub async fn config(&self) -> Result<Config, AppError> {
        match self.call(Call::GetConfig).await? {
            Response::Config {
                download_folder,
                default_folder,
                speed_limit,
                max_active,
            } => Ok(Config {
                download_folder,
                default_folder,
                speed_limit,
                max_active,
            }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    pub async fn set_config(&self, change: ConfigChange) -> Result<Config, AppError> {
        match self
            .call(Call::SetConfig {
                download_folder: change.download_folder,
                speed_limit: change.speed_limit,
                max_active: change.max_active,
            })
            .await?
        {
            Response::Config {
                download_folder,
                default_folder,
                speed_limit,
                max_active,
            } => Ok(Config {
                download_folder,
                default_folder,
                speed_limit,
                max_active,
            }),
            _ => Err(AppError::Engine("unexpected answer from the engine".into())),
        }
    }

    /// The browser extension's settings (shared with the browsers).
    pub async fn extension_settings(&self) -> Result<ExtensionSettings, AppError> {
        let text = match self
            .call(Call::SettingGet {
                key: EXTENSION_KEY.into(),
            })
            .await?
        {
            Response::Setting { value } => value,
            _ => return Err(AppError::Engine("unexpected answer from the engine".into())),
        };
        Ok(ExtensionSettings::from_text(text.as_deref()))
    }

    /// Saves new extension settings. The browsers take them over at their next check.
    pub async fn set_extension_settings(
        &self,
        new: ExtensionSettings,
    ) -> Result<ExtensionSettings, AppError> {
        let current = match self
            .call(Call::SettingGet {
                key: EXTENSION_KEY.into(),
            })
            .await?
        {
            Response::Setting { value } => value,
            _ => return Err(AppError::Engine("unexpected answer from the engine".into())),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let text = merge_extension(current.as_deref(), &new, now);
        self.call(Call::SettingSet {
            key: EXTENSION_KEY.into(),
            value: text.clone(),
        })
        .await?;
        Ok(ExtensionSettings::from_text(Some(&text)))
    }

    pub async fn pause(&self, id: i64) -> Result<(), AppError> {
        self.call(Call::Pause { id }).await.map(|_| ())
    }

    pub async fn resume(&self, id: i64) -> Result<(), AppError> {
        self.call(Call::Resume { id }).await.map(|_| ())
    }

    pub async fn cancel(&self, id: i64) -> Result<(), AppError> {
        self.call(Call::Cancel { id }).await.map(|_| ())
    }

    pub async fn remove(&self, id: i64, delete_file: bool) -> Result<(), AppError> {
        self.call(Call::Remove { id, delete_file })
            .await
            .map(|_| ())
    }
}

/// Starts `mohona daemon` in the background, not tied to the app's window.
fn spawn(
    program: &std::path::Path,
    info: &std::path::Path,
    database: Option<&std::path::Path>,
) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(program);
    if let Some(database) = database {
        command.arg("--db").arg(database);
    }
    command
        .arg("daemon")
        .arg("--info")
        .arg(info)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window, and not part of the app's process group.
        command.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    command.spawn().map(|_| ())
}

/// Where the `mohona` program is: the explicit setting, else next to `exe`,
/// else the first folder of `path` that has it.
pub fn find_program(
    setting: Option<std::ffi::OsString>,
    exe: Option<&std::path::Path>,
    path: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "mohona.exe"
    } else {
        "mohona"
    };
    if let Some(p) = setting.map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    if let Some(p) = exe
        .and_then(|e| e.parent())
        .map(|d| d.join(name))
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    std::env::split_paths(&path?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_program_in_the_order_setting_then_next_to_the_app_then_path() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "mohona.exe"
        } else {
            "mohona"
        };
        let (a, b, c) = (
            dir.path().join("a"),
            dir.path().join("b"),
            dir.path().join("c"),
        );
        for d in [&a, &b, &c] {
            std::fs::create_dir(d).unwrap();
        }
        std::fs::write(a.join("custom"), "").unwrap();
        std::fs::write(b.join(name), "").unwrap();
        std::fs::write(c.join(name), "").unwrap();
        let exe = b.join("app");
        let path = std::env::join_paths([&c]).unwrap();
        assert_eq!(
            find_program(
                Some(a.join("custom").into()),
                Some(&exe),
                Some(path.clone())
            ),
            Some(a.join("custom"))
        );
        assert_eq!(
            find_program(
                Some(a.join("missing").into()),
                Some(&exe),
                Some(path.clone())
            ),
            Some(b.join(name))
        );
        assert_eq!(
            find_program(None, Some(&a.join("app")), Some(path)),
            Some(c.join(name))
        );
        assert_eq!(find_program(None, None, None), None);
    }
}
