//! What to do when the downloads are finished: close the app, put the
//! computer to sleep or off, or run a program.
//!
//! The manager never does any of this without a countdown that the person
//! can see (as [`ManagerEvent::ActionCountdown`](crate::ManagerEvent)
//! events) and cancel. The operating-system part sits behind
//! [`ActionRunner`] so tests can use a fake one.

use std::process::{Command, Stdio};

/// What to do after the last download finishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterAction {
    /// Tell the application to close (the manager only reports it).
    Exit,
    /// Put the computer to sleep.
    Sleep,
    /// Hibernate the computer (sleep where the system has no hibernation).
    Hibernate,
    /// Shut the computer down.
    Shutdown,
    /// Start a program with some arguments (no shell is involved).
    Run {
        /// The program to start.
        program: String,
        /// Its arguments.
        args: Vec<String>,
    },
}

impl AfterAction {
    /// Read `exit`, `sleep`, `hibernate`, `shutdown` or `run:<program> [args]`.
    pub fn parse(text: &str) -> Result<AfterAction, String> {
        let text = text.trim();
        match text.to_ascii_lowercase().as_str() {
            "exit" => return Ok(AfterAction::Exit),
            "sleep" => return Ok(AfterAction::Sleep),
            "hibernate" => return Ok(AfterAction::Hibernate),
            "shutdown" => return Ok(AfterAction::Shutdown),
            _ => {}
        }
        if let Some(command) = text.strip_prefix("run:") {
            let mut words = command.split_whitespace().map(str::to_string);
            let program = words.next().ok_or("run: needs a program")?;
            return Ok(AfterAction::Run {
                program,
                args: words.collect(),
            });
        }
        Err(format!(
            "'{text}' is not an action; use exit, sleep, hibernate, shutdown or run:<program>"
        ))
    }

    /// The word that [`AfterAction::parse`] reads back (`run` actions keep only the program).
    pub fn name(&self) -> &str {
        match self {
            AfterAction::Exit => "exit",
            AfterAction::Sleep => "sleep",
            AfterAction::Hibernate => "hibernate",
            AfterAction::Shutdown => "shutdown",
            AfterAction::Run { .. } => "run",
        }
    }

    /// A short phrase for the countdown ("shutting down").
    pub fn describe(&self) -> String {
        match self {
            AfterAction::Exit => "closing".into(),
            AfterAction::Sleep => "going to sleep".into(),
            AfterAction::Hibernate => "hibernating".into(),
            AfterAction::Shutdown => "shutting down".into(),
            AfterAction::Run { program, .. } => format!("running {program}"),
        }
    }
}

/// Carries an action out.
pub trait ActionRunner: Send + Sync {
    /// Do it. An error is reported to the person; nothing is retried.
    fn run(&self, action: &AfterAction) -> Result<(), String>;
}

/// The real thing: asks the operating system.
#[derive(Debug, Default)]
pub struct SystemRunner;

fn spawn(program: &str, args: &[&str]) -> Result<(), String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start {program}: {e}"))
}

impl ActionRunner for SystemRunner {
    fn run(&self, action: &AfterAction) -> Result<(), String> {
        match action {
            // The application closes itself when it sees the event.
            AfterAction::Exit => Ok(()),
            AfterAction::Run { program, args } => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                spawn(program, &args)
            }
            AfterAction::Shutdown => {
                if cfg!(windows) {
                    spawn("shutdown", &["/s", "/t", "0"])
                } else if cfg!(target_os = "macos") {
                    spawn(
                        "osascript",
                        &["-e", "tell application \"System Events\" to shut down"],
                    )
                } else {
                    spawn("systemctl", &["poweroff"])
                }
            }
            AfterAction::Sleep => {
                if cfg!(windows) {
                    spawn("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"])
                } else if cfg!(target_os = "macos") {
                    spawn("pmset", &["sleepnow"])
                } else {
                    spawn("systemctl", &["suspend"])
                }
            }
            AfterAction::Hibernate => {
                if cfg!(windows) {
                    spawn("shutdown", &["/h"])
                } else if cfg!(target_os = "macos") {
                    spawn("pmset", &["sleepnow"])
                } else {
                    spawn("systemctl", &["hibernate"])
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_read() {
        assert_eq!(AfterAction::parse("Shutdown"), Ok(AfterAction::Shutdown));
        assert_eq!(AfterAction::parse(" sleep "), Ok(AfterAction::Sleep));
        assert_eq!(AfterAction::parse("exit"), Ok(AfterAction::Exit));
        assert_eq!(
            AfterAction::parse("run:notify-send done now"),
            Ok(AfterAction::Run {
                program: "notify-send".into(),
                args: vec!["done".into(), "now".into()]
            })
        );
        assert!(AfterAction::parse("run:").is_err());
        assert!(AfterAction::parse("explode").is_err());
    }
}
