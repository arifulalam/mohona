//! Stream downloads (HLS and DASH): run by the `mohona stream` command.
//!
//! The stream downloader lives in the command line program, so the daemon
//! runs that program for each job, with the browser's context passed as
//! separate arguments (never through a shell). Values are given as
//! `--option=value` and the address comes after `--`, so nothing a web page
//! controls can turn into another option.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::protocol::JobInfo;

const KEEP_JOBS: usize = 100;

/// What to download and how.
pub struct StreamJob {
    pub url: String,
    pub folder: PathBuf,
    pub cookies: Option<String>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    pub headers: Vec<(String, String)>,
    pub name: Option<String>,
}

#[derive(Clone)]
pub struct Streams {
    program: Option<PathBuf>,
    jobs: Arc<Mutex<Vec<JobInfo>>>,
}

fn clean(text: &str) -> bool {
    !text.chars().any(char::is_control)
}

impl Streams {
    pub fn new(program: Option<PathBuf>) -> Self {
        Self {
            program,
            jobs: Arc::default(),
        }
    }

    pub fn list(&self) -> Vec<JobInfo> {
        self.jobs.lock().map(|j| j.clone()).unwrap_or_default()
    }

    /// The command line for a job (public for tests). The session cookie is
    /// not here: it travels in the environment (`MOHONA_COOKIE`), where other
    /// programs cannot read it from the process list.
    pub fn arguments(job: &StreamJob) -> Vec<String> {
        let mut args = vec![
            "stream".to_string(),
            format!("--output={}", job.folder.display()),
        ];
        if let Some(v) = &job.name {
            args.push(format!("--name={v}"));
        }
        if let Some(v) = &job.referrer {
            args.push(format!("--referer={v}"));
        }
        if let Some(v) = &job.user_agent {
            args.push(format!("--user-agent={v}"));
        }
        for (name, value) in &job.headers {
            args.push(format!("--header={name}: {value}"));
        }
        args.push("--".into());
        args.push(job.url.clone());
        args
    }

    /// `env`: extra variables for the program (where ffmpeg is, as the person chose).
    pub fn start(&self, env: Vec<(String, String)>, job: StreamJob) -> Result<u64, String> {
        let Some(program) = &self.program else {
            return Err("stream downloads are not available in this daemon".into());
        };
        let lower = job.url.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err("only http and https streams can be downloaded".into());
        }
        let all_clean = clean(&job.url)
            && job
                .cookies
                .iter()
                .chain(&job.referrer)
                .chain(&job.user_agent)
                .chain(&job.name)
                .all(|v| clean(v))
            && job
                .headers
                .iter()
                .all(|(n, v)| clean(n) && clean(v) && !n.contains(':'));
        if !all_clean {
            return Err("an address or header contains control characters".into());
        }
        if job
            .name
            .as_deref()
            .is_some_and(|n| n.contains(['/', '\\', ':']) || n.starts_with('.'))
        {
            return Err("the name must be a plain file name".into());
        }
        let mut command = tokio::process::Command::new(program);
        command.args(Self::arguments(&job));
        command.envs(env);
        if let Some(cookies) = &job.cookies {
            command.env("MOHONA_COOKIE", cookies);
        }
        let url = job.url;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", program.display()))?;
        let id = {
            let mut jobs = self.jobs.lock().map_err(|_| "internal error".to_string())?;
            let id = jobs.last().map_or(1, |j| j.id + 1);
            jobs.push(JobInfo {
                id,
                url,
                status: "running".into(),
                message: None,
            });
            if jobs.len() > KEEP_JOBS {
                jobs.remove(0);
            }
            id
        };
        let jobs = Arc::clone(&self.jobs);
        let stderr = child.stderr.take();
        tokio::spawn(async move {
            let mut last = None;
            if let Some(stderr) = stderr {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        last = Some(line);
                    }
                }
            }
            let ok = child.wait().await.is_ok_and(|s| s.success());
            if let Ok(mut jobs) = jobs.lock() {
                if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
                    job.status = if ok { "finished" } else { "failed" }.into();
                    job.message = if ok { None } else { last };
                }
            }
        });
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> StreamJob {
        StreamJob {
            url: "https://a/--output=/etc".into(),
            folder: "/dl".into(),
            cookies: Some("--insecure".into()),
            referrer: Some("-x".into()),
            user_agent: None,
            headers: vec![("X".into(), "--proxy=evil".into())],
            name: Some("--evil".into()),
        }
    }

    #[test]
    fn values_are_attached_to_their_option_and_the_address_comes_last() {
        assert_eq!(
            Streams::arguments(&job()),
            [
                "stream",
                "--output=/dl",
                "--name=--evil",
                "--referer=-x",
                "--header=X: --proxy=evil",
                "--",
                "https://a/--output=/etc"
            ]
        );
    }

    #[tokio::test]
    async fn without_a_program_nothing_starts() {
        assert!(Streams::new(None).start(Vec::new(), job()).is_err());
    }

    #[tokio::test]
    async fn a_name_with_a_path_is_refused() {
        let streams = Streams::new(Some("mohona".into()));
        for name in ["../x", "a/b", "a\\b", "C:x", ".hidden"] {
            let mut j = job();
            j.name = Some(name.into());
            j.url = "https://a/x.m3u8".into();
            assert!(streams.start(Vec::new(), j).is_err(), "{name}");
        }
    }
}
