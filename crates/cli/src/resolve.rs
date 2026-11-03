//! `mohona resolve URL`: the direct address of a cloud share link.

use std::process::ExitCode;

pub fn run(url: &str) -> ExitCode {
    match mohona_daemon::cloud::resolve(url) {
        Some(link) => {
            println!("{}: {}", link.service, link.url);
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("Not a share link Mohona knows (Google Drive, Google Docs, Dropbox, OneDrive, GitHub). It can still be downloaded as it is.");
            ExitCode::FAILURE
        }
    }
}
