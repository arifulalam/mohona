//! `mohona history` and `mohona log`: finding old downloads.

use chrono::{Local, NaiveDate, TimeZone};
use mohona_store::{HistoryQuery, Store};

use crate::format_bytes;

/// A date like 2026-10-17 as the unix time of the start of that day, here.
pub fn parse_date(text: &str) -> Result<i64, String> {
    let date = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| format!("{text:?} is not a date like 2026-10-17"))?;
    let midnight = date.and_hms_opt(0, 0, 0).expect("midnight exists");
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map(|t| t.timestamp())
        .ok_or_else(|| format!("{text:?} does not exist in this time zone"))
}

fn when(unix: i64) -> String {
    Local
        .timestamp_opt(unix, 0)
        .single()
        .map_or_else(|| "?".into(), |t| t.format("%Y-%m-%d %H:%M").to_string())
}

pub struct Filters {
    pub text: Vec<String>,
    pub status: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub category: Option<String>,
    pub limit: u32,
}

pub fn history(store: &Store, filters: Filters) -> Result<(), String> {
    let status = match &filters.status {
        Some(text) => Some(mohona_store::Status::parse(text).ok_or(format!(
            "unknown status {text:?}: use queued, probing, downloading, paused, retrying, completed, failed or canceled"
        ))?),
        None => None,
    };
    let text = (!filters.text.is_empty()).then(|| filters.text.join(" "));
    let rows = store
        .search(&HistoryQuery {
            text,
            status,
            since: filters.since,
            // `--until 2026-10-17` includes that whole day.
            until: filters.until.map(|t| t + 86_400),
            category: filters.category,
            limit: Some(filters.limit),
        })
        .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        println!("Nothing found.");
        return Ok(());
    }
    for row in &rows {
        let name = row.filename.as_deref().unwrap_or(&row.url);
        let size = row.size.map_or("?".to_string(), format_bytes);
        println!(
            "#{:<3} {}  {:<11} {:>10}  {}",
            row.id,
            when(row.created_at),
            row.status.as_str(),
            size,
            name
        );
    }
    if rows.len() as u32 == filters.limit {
        println!(
            "(showing the newest {}; use --limit for more)",
            filters.limit
        );
    }
    Ok(())
}

pub fn log(store: &Store, id: i64) -> Result<(), String> {
    store
        .download(id)
        .map_err(|e| e.to_string())?
        .ok_or(format!("no download #{id}"))?;
    let lines = store.log_of(id).map_err(|e| e.to_string())?;
    if lines.is_empty() {
        println!("Nothing logged for #{id}.");
    }
    for line in lines {
        println!("{}  {:<5} {}", when(line.at), line.level, line.message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_parse() {
        let a = parse_date("2026-10-17").unwrap();
        let b = parse_date("2026-10-18").unwrap();
        assert!((86_400 - 3600..=86_400 + 3600).contains(&(b - a)));
        assert!(parse_date("17/10/2026").is_err());
        assert!(parse_date("2026-13-01").is_err());
    }
}
