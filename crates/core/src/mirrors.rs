//! Several addresses for one file.

use mohona_net::{HttpClient, NetError, Probe};
use tokio::sync::mpsc::Sender;
use tokio::task::JoinSet;

use crate::download::{emit, DownloadRequest, Event};
use crate::parallel::Source;
use crate::retry::probe_with_retry;

/// The mirror addresses worth trying: trimmed, not empty, not the main
/// address, each once.
fn mirror_addresses(request: &DownloadRequest) -> Vec<String> {
    let main = request.url.trim();
    let mut found: Vec<String> = Vec::new();
    for address in &request.context.mirrors {
        let address = address.trim();
        if !address.is_empty() && address != main && !found.iter().any(|a| a == address) {
            found.push(address.to_string());
        }
    }
    found
}

/// Probe the main address. If it cannot be reached, the first mirror that
/// answers takes its place. Returns what was learned and the mirrors that
/// were not probed yet.
pub(crate) async fn probe_main(
    client: &HttpClient,
    request: &DownloadRequest,
    events: Option<&Sender<Event>>,
) -> Result<(Probe, Vec<String>), NetError> {
    let mirrors = mirror_addresses(request);
    let first_error = match probe_with_retry(client, &request.url, &request.retry).await {
        Ok(probe) => return Ok((probe, mirrors)),
        Err(error) => error,
    };
    if mirrors.is_empty() {
        return Err(first_error);
    }
    emit(
        events,
        Event::Note(format!(
            "{} did not answer ({first_error}); trying the mirrors",
            request.url.trim()
        )),
    )
    .await;
    for (index, mirror) in mirrors.iter().enumerate() {
        match probe_with_retry(client, mirror, &request.retry).await {
            Ok(probe) => {
                emit(events, Event::Note(format!("using the mirror {mirror}"))).await;
                return Ok((probe, mirrors[index + 1..].to_vec()));
            }
            Err(error) => {
                emit(
                    events,
                    Event::Note(format!("the mirror {mirror} did not answer ({error})")),
                )
                .await;
            }
        }
    }
    Err(first_error)
}

/// Check the other addresses at the same time, and keep those that hold the
/// same file as far as can be told (same size, and byte ranges work).
pub(crate) async fn usable(
    client: &HttpClient,
    candidates: &[String],
    total: u64,
    events: Option<&Sender<Event>>,
) -> Vec<Source> {
    let mut checks = JoinSet::new();
    for (index, address) in candidates.iter().enumerate() {
        let client = client.clone();
        let address = address.clone();
        checks.spawn(async move { (index, address.clone(), client.probe(&address).await) });
    }
    let mut results = Vec::new();
    while let Some(done) = checks.join_next().await {
        if let Ok(done) = done {
            results.push(done);
        }
    }
    results.sort_by_key(|(index, _, _)| *index);
    let mut sources = Vec::new();
    for (_, address, result) in results {
        let skipped = match result {
            Ok(probe) if probe.size != Some(total) => Some(format!(
                "it has a different size ({})",
                probe
                    .size
                    .map_or_else(|| "unknown".to_string(), |size| format!("{size} bytes"))
            )),
            Ok(probe) if !probe.accepts_ranges => Some("it cannot send parts of the file".into()),
            Ok(probe) => {
                sources.push(Source {
                    url: probe.final_url,
                    etag: probe.etag,
                });
                None
            }
            Err(error) => Some(error.to_string()),
        };
        if let Some(reason) = skipped {
            emit(
                events,
                Event::Note(format!("the mirror {address} is not used: {reason}")),
            )
            .await;
        }
    }
    sources
}
