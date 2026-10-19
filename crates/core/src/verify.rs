//! Checking a finished file against a checksum.

use std::path::Path;

use mohona_net::{hash_file, Algorithm, Checksum, HttpClient, Probe};
use tokio::sync::mpsc::Sender;

use crate::download::{emit, Event};
use crate::Error;

/// Where the checksum that was checked came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The person gave it.
    Given,
    /// A checksum file next to the download, at this address.
    File(String),
    /// The server announced it in its headers.
    Header,
}

/// A checksum the finished file passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// What the file was checked against.
    pub checksum: Checksum,
    /// Where that came from.
    pub source: Source,
}

/// The neighbors tried, strongest first.
const NEIGHBORS: [Algorithm; 4] = [
    Algorithm::Sha256,
    Algorithm::Sha512,
    Algorithm::Sha1,
    Algorithm::Md5,
];

/// Checksum files are tiny; anything bigger is not one.
const MAX_CHECKSUM_FILE: usize = 64 * 1024;

/// Decide what the file should be checked against, if anything.
pub(crate) async fn expected(
    client: &HttpClient,
    context: &mohona_net::RequestContext,
    probe: &Probe,
    filename: &str,
    events: Option<&Sender<Event>>,
) -> Option<(Checksum, Source)> {
    if let Some(given) = &context.checksum {
        return Some((given.clone(), Source::Given));
    }
    if context.find_checksum {
        if let Some(found) = find_neighbor(client, &probe.final_url, filename).await {
            return Some(found);
        }
        emit(
            events,
            Event::Note("no checksum file found next to the download".into()),
        )
        .await;
    }
    probe
        .digests
        .iter()
        .max_by_key(|c| c.algorithm)
        .map(|c| (c.clone(), Source::Header))
}

async fn find_neighbor(
    client: &HttpClient,
    address: &str,
    filename: &str,
) -> Option<(Checksum, Source)> {
    // The address without its query: `…/a.iso?x=1` has `…/a.iso.sha256`.
    let base = address.split(['?', '#']).next().unwrap_or(address);
    for algorithm in NEIGHBORS {
        let url = format!("{base}.{}", algorithm.name());
        let Ok(mut body) = client.get(&url).await else {
            continue;
        };
        let mut text = Vec::new();
        while let Ok(Some(chunk)) = body.chunk().await {
            text.extend_from_slice(&chunk);
            if text.len() > MAX_CHECKSUM_FILE {
                break;
            }
        }
        if text.len() > MAX_CHECKSUM_FILE {
            continue;
        }
        let text = String::from_utf8_lossy(&text);
        if let Some(found) = Checksum::from_checksum_file(&text, filename, algorithm) {
            return Some((found, Source::File(url)));
        }
    }
    None
}

/// Hash the file and compare. A mismatch keeps the file where it is.
pub(crate) async fn check(
    path: &Path,
    expected: Checksum,
    source: Source,
    events: Option<&Sender<Event>>,
) -> Result<Verified, Error> {
    emit(
        events,
        Event::Note(format!("checking {}", expected.algorithm.name())),
    )
    .await;
    let owned = path.to_path_buf();
    let algorithm = expected.algorithm;
    let actual = tokio::task::spawn_blocking(move || hash_file(&owned, algorithm))
        .await
        .map_err(std::io::Error::other)??;
    if actual == expected.hex {
        Ok(Verified {
            checksum: expected,
            source,
        })
    } else {
        Err(Error::ChecksumMismatch {
            algorithm: algorithm.name().to_string(),
            expected: expected.hex,
            actual,
            path: path.to_path_buf(),
        })
    }
}
