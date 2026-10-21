//! HLS (HTTP Live Streaming) playlists, RFC 8216.
//!
//! [`parse`] reads either kind of playlist. A *master* playlist offers
//! variants (qualities) and separate audio or subtitle tracks; a *media*
//! playlist lists the segments of one rendition.

use url::Url;

use crate::error::MediaError;
use crate::model::{
    ByteRange, Encryption, InitSegment, MediaKind, MediaVariant, Resolution, Segment, SegmentList,
    Track,
};

/// A parsed playlist.
#[derive(Debug, Clone, PartialEq)]
pub enum Playlist {
    Master {
        variants: Vec<MediaVariant>,
        tracks: Vec<Track>,
    },
    Media(SegmentList),
}

/// Reads a playlist. `base` is the address it was fetched from; relative
/// addresses inside it are resolved against it.
pub fn parse(text: &str, base: &Url) -> Result<Playlist, MediaError> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if !lines.next().is_some_and(|l| l.starts_with("#EXTM3U")) {
        return Err(MediaError::NotAPlaylist);
    }
    let lines: Vec<&str> = lines.collect();
    let is_master = lines.iter().any(|l| l.starts_with("#EXT-X-STREAM-INF:"));
    if is_master {
        parse_master(&lines, base)
    } else {
        parse_media(&lines, base).map(Playlist::Media)
    }
}

fn join(base: &Url, reference: &str) -> Result<Url, MediaError> {
    base.join(reference)
        .map_err(|e| MediaError::Invalid(format!("bad address \"{reference}\": {e}")))
}

/// Splits `KEY=value,KEY="quoted, value"` into pairs, unquoting values.
fn attributes(list: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut rest = list;
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_string();
        let after = &rest[eq + 1..];
        let (value, remaining) = if let Some(quoted) = after.strip_prefix('"') {
            match quoted.find('"') {
                Some(end) => (&quoted[..end], quoted[end + 1..].trim_start_matches(',')),
                None => (quoted, ""),
            }
        } else {
            match after.find(',') {
                Some(end) => (&after[..end], &after[end + 1..]),
                None => (after, ""),
            }
        };
        pairs.push((key, value.trim().to_string()));
        rest = remaining;
    }
    pairs
}

fn get<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn parse_master(lines: &[&str], base: &Url) -> Result<Playlist, MediaError> {
    let mut variants = Vec::new();
    let mut tracks = Vec::new();
    let mut pending: Option<Vec<(String, String)>> = None;
    for line in lines {
        if let Some(list) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(attributes(list));
        } else if let Some(list) = line.strip_prefix("#EXT-X-MEDIA:") {
            let attrs = attributes(list);
            let kind = match get(&attrs, "TYPE") {
                Some("AUDIO") => MediaKind::Audio,
                Some("SUBTITLES") => MediaKind::Subtitles,
                _ => continue,
            };
            let url = get(&attrs, "URI").map(|u| join(base, u)).transpose()?;
            tracks.push(Track {
                kind,
                group: get(&attrs, "GROUP-ID").unwrap_or_default().to_string(),
                name: get(&attrs, "NAME").unwrap_or_default().to_string(),
                language: get(&attrs, "LANGUAGE").map(str::to_string),
                default: get(&attrs, "DEFAULT") == Some("YES"),
                url,
            });
        } else if !line.starts_with('#') {
            if let Some(attrs) = pending.take() {
                let bandwidth = get(&attrs, "BANDWIDTH")
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| MediaError::Invalid("a variant has no BANDWIDTH".into()))?;
                variants.push(MediaVariant {
                    url: join(base, line)?,
                    bandwidth,
                    average_bandwidth: get(&attrs, "AVERAGE-BANDWIDTH")
                        .and_then(|v| v.parse().ok()),
                    resolution: get(&attrs, "RESOLUTION").and_then(parse_resolution),
                    frame_rate: get(&attrs, "FRAME-RATE").and_then(|v| v.parse().ok()),
                    codecs: get(&attrs, "CODECS").map(str::to_string),
                    audio_group: get(&attrs, "AUDIO").map(str::to_string),
                    subtitle_group: get(&attrs, "SUBTITLES").map(str::to_string),
                });
            }
        }
    }
    if variants.is_empty() {
        return Err(MediaError::Empty);
    }
    Ok(Playlist::Master { variants, tracks })
}

fn parse_resolution(value: &str) -> Option<Resolution> {
    let (w, h) = value.split_once('x')?;
    Some(Resolution {
        width: w.parse().ok()?,
        height: h.parse().ok()?,
    })
}

/// `length` or `length@offset`.
fn parse_range(value: &str, next_offset: Option<u64>) -> Result<ByteRange, MediaError> {
    let bad = || MediaError::Invalid(format!("bad byte range \"{value}\""));
    let (length, offset) = match value.split_once('@') {
        Some((l, o)) => (l, Some(o.parse::<u64>().map_err(|_| bad())?)),
        None => (value, None),
    };
    Ok(ByteRange {
        length: length.parse().map_err(|_| bad())?,
        offset: offset.or(next_offset).unwrap_or(0),
    })
}

fn parse_iv(value: &str) -> Result<[u8; 16], MediaError> {
    let bad = || MediaError::Invalid(format!("bad IV \"{value}\""));
    let hex = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(bad)?;
    if hex.len() != 32 || !hex.is_ascii() {
        return Err(bad());
    }
    let mut iv = [0u8; 16];
    for (i, byte) in iv.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| bad())?;
    }
    Ok(iv)
}

fn parse_key(list: &str, base: &Url) -> Result<Option<Encryption>, MediaError> {
    let attrs = attributes(list);
    let method = get(&attrs, "METHOD").unwrap_or("NONE");
    if let Some(format) = get(&attrs, "KEYFORMAT") {
        if format != "identity" {
            return Err(MediaError::Protected(format!("key format {format}")));
        }
    }
    match method {
        "NONE" => Ok(None),
        "AES-128" => {
            let uri = get(&attrs, "URI")
                .ok_or_else(|| MediaError::Invalid("AES-128 key without URI".into()))?;
            Ok(Some(Encryption::Aes128 {
                key_url: join(base, uri)?,
                iv: get(&attrs, "IV").map(parse_iv).transpose()?,
            }))
        }
        other => Err(MediaError::Protected(format!("encryption method {other}"))),
    }
}

fn parse_media(lines: &[&str], base: &Url) -> Result<SegmentList, MediaError> {
    let mut segments = Vec::new();
    let mut ended = false;
    let mut target_duration = 0.0;
    let mut sequence: u64 = 0;
    let mut encryption: Option<Encryption> = None;
    let mut init: Option<InitSegment> = None;
    let mut duration: Option<f64> = None;
    let mut range: Option<ByteRange> = None;
    let mut discontinuity = false;
    let mut last_range_end: Option<(Url, u64)> = None;

    for line in lines {
        if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            target_duration = value.trim().parse().unwrap_or(0.0);
        } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            sequence = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            let seconds = value.split(',').next().unwrap_or("").trim();
            duration = Some(
                seconds
                    .parse()
                    .map_err(|_| MediaError::Invalid(format!("bad duration \"{seconds}\"")))?,
            );
        } else if let Some(value) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            // The offset may be left out; it then continues the previous piece.
            range = Some(parse_range(value.trim(), None)?);
            // Resolved against the previous segment when the URI is known.
            if !value.contains('@') {
                range = Some(ByteRange {
                    offset: u64::MAX,
                    ..range.unwrap()
                });
            }
        } else if let Some(list) = line.strip_prefix("#EXT-X-KEY:") {
            encryption = parse_key(list, base)?;
        } else if let Some(list) = line.strip_prefix("#EXT-X-MAP:") {
            let attrs = attributes(list);
            let uri = get(&attrs, "URI")
                .ok_or_else(|| MediaError::Invalid("EXT-X-MAP without URI".into()))?;
            init = Some(InitSegment {
                url: join(base, uri)?,
                range: get(&attrs, "BYTERANGE")
                    .map(|r| parse_range(r, None))
                    .transpose()?,
            });
        } else if line.starts_with("#EXT-X-DISCONTINUITY-SEQUENCE") {
            // Only matters for players that keep timelines across reloads.
        } else if line.starts_with("#EXT-X-DISCONTINUITY") {
            discontinuity = true;
        } else if line.starts_with("#EXT-X-ENDLIST") {
            ended = true;
        } else if line.starts_with("#EXT-X-SESSION-KEY") {
            // Master-only tag; ignore.
        } else if !line.starts_with('#') {
            let Some(seconds) = duration.take() else {
                // A URI without #EXTINF: not a segment.
                continue;
            };
            let url = join(base, line)?;
            let range = range.take().map(|mut r| {
                if r.offset == u64::MAX {
                    r.offset = match &last_range_end {
                        Some((previous, end)) if *previous == url => *end,
                        _ => 0,
                    };
                }
                r
            });
            if let Some(r) = range {
                last_range_end = Some((url.clone(), r.offset + r.length));
            }
            segments.push(Segment {
                sequence,
                url,
                range,
                duration: seconds,
                encryption: encryption.clone(),
                init: init.clone(),
                discontinuity: std::mem::take(&mut discontinuity),
            });
            sequence += 1;
        }
    }
    if segments.is_empty() {
        return Err(MediaError::Empty);
    }
    Ok(SegmentList {
        segments,
        ended,
        target_duration,
    })
}
