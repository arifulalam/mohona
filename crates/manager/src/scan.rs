//! A virus scan of each finished download, by the Windows Defender command line or by a program
//! the person names. Mohona never deletes or moves the file: it only says what the scanner said.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The setting that says how to scan: `off` (the default), `defender` or `custom`.
pub const MODE: &str = "scan.mode";
/// The setting with the program and its arguments for `custom`; `{file}` stands for the file
/// (without it the file is added at the end).
pub const COMMAND: &str = "scan.command";

/// How long a scan may take.
const LIMIT: Duration = Duration::from_secs(10 * 60);

/// What the scanner said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing found.
    Clean,
    /// The scanner reported a problem (its words, or its exit code).
    Threat(String),
    /// The scan could not be done (no scanner, it did not start, it took too long).
    NotRun(String),
}

/// A program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scanner {
    program: PathBuf,
    args: Vec<String>,
}

/// Splits a command line into words: spaces separate, double quotes group, nothing else is special
/// (a Windows path keeps its backslashes).
pub fn split_command(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    words
}

/// The Windows Defender command line tool, if this computer has it.
pub fn defender() -> Option<PathBuf> {
    let program_files =
        std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into());
    let classic = PathBuf::from(program_files)
        .join("Windows Defender")
        .join("MpCmdRun.exe");
    // Newer Windows keeps the newest copy under ProgramData, in a folder named by version.
    let newest = std::env::var_os("ProgramData").and_then(|data| {
        let platform = PathBuf::from(data)
            .join("Microsoft")
            .join("Windows Defender")
            .join("Platform");
        let mut versions: Vec<PathBuf> = std::fs::read_dir(platform)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path().join("MpCmdRun.exe"))
            .filter(|p| p.is_file())
            .collect();
        versions.sort();
        versions.pop()
    });
    newest.or_else(|| classic.is_file().then_some(classic))
}

impl Scanner {
    /// The scanner the settings ask for: `None` when scanning is off, `Err` when it is on but
    /// cannot work (the reason is for the person).
    pub fn from_settings(
        mode: Option<&str>,
        command: Option<&str>,
    ) -> Result<Option<Self>, String> {
        match mode.map(str::trim) {
            None | Some("" | "off") => Ok(None),
            Some("defender") => {
                let program = defender()
                    .ok_or("Windows Defender's command line tool (MpCmdRun.exe) was not found")?;
                Ok(Some(Self {
                    program,
                    args: [
                        "-Scan",
                        "-ScanType",
                        "3",
                        "-DisableRemediation",
                        "-File",
                        "{file}",
                    ]
                    .map(String::from)
                    .to_vec(),
                }))
            }
            Some("custom") => {
                let mut words = split_command(command.unwrap_or(""));
                if words.is_empty() {
                    return Err("no scanner program is set".into());
                }
                let program = PathBuf::from(words.remove(0));
                if !words.iter().any(|w| w.contains("{file}")) {
                    words.push("{file}".into());
                }
                Ok(Some(Self {
                    program,
                    args: words,
                }))
            }
            Some(other) => Err(format!("unknown scan mode \"{other}\"")),
        }
    }

    /// Scans a file or folder.
    pub async fn scan(&self, path: &Path) -> Verdict {
        let file = path.display().to_string();
        let mut command = tokio::process::Command::new(&self.program);
        command
            .args(self.args.iter().map(|a| a.replace("{file}", &file)))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // no console window
        let run = tokio::time::timeout(LIMIT, command.output()).await;
        match run {
            Err(_) => Verdict::NotRun("the scan took too long and was stopped".into()),
            Ok(Err(e)) => Verdict::NotRun(format!(
                "{} could not be started: {e}",
                self.program.display()
            )),
            Ok(Ok(out)) if out.status.success() => Verdict::Clean,
            Ok(Ok(out)) => {
                let said = String::from_utf8_lossy(&out.stdout);
                let said = said
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("");
                let code = out
                    .status
                    .code()
                    .map_or_else(|| "?".into(), |c| c.to_string());
                Verdict::Threat(if said.is_empty() {
                    format!("the scanner reported a problem (exit code {code})")
                } else {
                    format!("the scanner reported a problem (exit code {code}): {said}")
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_are_split_like_a_person_would() {
        assert_eq!(
            split_command(r#""C:\Program Files\AV\scan.exe" --deep  {file}"#),
            vec![r"C:\Program Files\AV\scan.exe", "--deep", "{file}"]
        );
        assert_eq!(split_command("  "), Vec::<String>::new());
        assert_eq!(split_command(r#"a "" b"#), vec!["a", "", "b"]);
    }

    #[test]
    fn the_settings_choose_the_scanner() {
        assert_eq!(Scanner::from_settings(None, None), Ok(None));
        assert_eq!(Scanner::from_settings(Some("off"), Some("x")), Ok(None));
        assert!(Scanner::from_settings(Some("custom"), Some("")).is_err());
        assert!(Scanner::from_settings(Some("what"), None).is_err());
        let s = Scanner::from_settings(Some("custom"), Some("scan --quick"))
            .unwrap()
            .unwrap();
        assert_eq!(s.args, vec!["--quick", "{file}"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_exit_code_says_clean_or_threat() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("scan.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\ncase \"$1\" in *evil*) echo 'Threat: Test.Virus'; exit 2;; esac\nexit 0\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let line = script.display().to_string();
        let scanner = Scanner::from_settings(Some("custom"), Some(&line))
            .unwrap()
            .unwrap();
        assert_eq!(
            scanner.scan(&dir.path().join("fine.bin")).await,
            Verdict::Clean
        );
        match scanner.scan(&dir.path().join("evil.bin")).await {
            Verdict::Threat(m) => {
                assert!(m.contains("Test.Virus") && m.contains("exit code 2"), "{m}")
            }
            other => panic!("{other:?}"),
        }
        let missing = Scanner::from_settings(Some("custom"), Some("/no/such/scanner"))
            .unwrap()
            .unwrap();
        assert!(matches!(
            missing.scan(&dir.path().join("a")).await,
            Verdict::NotRun(_)
        ));
    }
}
