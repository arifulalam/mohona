//! Subtitle text: WebVTT and TTML to SRT.
//!
//! SRT is the format every player and ffmpeg understand. Segmented WebVTT
//! (HLS) arrives as several files glued together, each with its own header;
//! [`vtt_to_srt`] accepts that, and drops cues that appear twice because they
//! cross a segment boundary.

use std::collections::HashSet;

use crate::error::MediaError;

/// One subtitle: start and end in milliseconds, and its text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cue {
    pub start: u64,
    pub end: u64,
    pub text: String,
}

/// What kind of subtitle text this is, judged by its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    WebVtt,
    Ttml,
}

pub fn detect(text: &str) -> Option<Format> {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("WEBVTT") {
        Some(Format::WebVtt)
    } else if t.starts_with('<') && t.contains("<tt") {
        Some(Format::Ttml)
    } else {
        None
    }
}

/// Converts WebVTT or TTML text to SRT, whichever it is.
pub fn to_srt(text: &str) -> Result<String, MediaError> {
    let cues = match detect(text) {
        Some(Format::WebVtt) => parse_vtt(text),
        Some(Format::Ttml) => parse_ttml(text)?,
        None => {
            return Err(MediaError::Invalid(
                "these subtitles are neither WebVTT nor TTML text (subtitles stored inside MP4 files are not supported yet)".into(),
            ))
        }
    };
    if cues.is_empty() {
        return Err(MediaError::Empty);
    }
    Ok(write_srt(&cues))
}

pub fn vtt_to_srt(text: &str) -> String {
    write_srt(&parse_vtt(text))
}

fn srt_time(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

pub fn write_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_time(cue.start),
            srt_time(cue.end),
            cue.text
        ));
    }
    out
}

/// `[hh:]mm:ss.mmm` in milliseconds.
fn vtt_time(token: &str) -> Option<u64> {
    let (clock, millis) = token.split_once(['.', ','])?;
    let mut parts: Vec<u64> = clock
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    if parts.len() == 2 {
        parts.insert(0, 0);
    }
    let fraction: String = millis.chars().take(3).collect();
    let ms: u64 = format!("{fraction:0<3}").parse().ok()?;
    Some((parts[0] * 3600 + parts[1] * 60 + parts[2]) * 1000 + ms)
}

fn decode_entities(text: &str) -> String {
    text.replace("&lrm;", "")
        .replace("&rlm;", "")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Keeps text and `<i>`, `<b>`, `<u>`; removes every other tag.
fn clean_markup(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = &rest[open + 1..open + close];
        let name = tag.trim_start_matches('/');
        if matches!(name, "i" | "b" | "u") {
            out.push('<');
            out.push_str(tag);
            out.push('>');
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    decode_entities(&out)
}

pub fn parse_vtt(text: &str) -> Vec<Cue> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    // Glued segment files do not always end with a blank line: a new
    // "WEBVTT" header always starts a new block.
    let mut separated = String::with_capacity(text.len() + 16);
    let mut previous_blank = true;
    for line in text.lines() {
        if line.starts_with("WEBVTT") && !previous_blank {
            separated.push('\n');
        }
        separated.push_str(line);
        separated.push('\n');
        previous_blank = line.trim().is_empty();
    }
    let text = separated;
    let mut cues = Vec::new();
    let mut seen: HashSet<Cue> = HashSet::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        let Some(first) = lines.first() else { continue };
        if ["WEBVTT", "NOTE", "STYLE", "REGION", "X-TIMESTAMP-MAP"]
            .iter()
            .any(|h| first.starts_with(h))
        {
            continue;
        }
        let Some(at) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let (from, rest) = lines[at].split_once("-->").expect("checked above");
        let to = rest.split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (vtt_time(from.trim()), vtt_time(to)) else {
            continue;
        };
        let body: Vec<String> = lines[at + 1..]
            .iter()
            .map(|l| clean_markup(l.trim()))
            .filter(|l| !l.trim().is_empty())
            .collect();
        if body.is_empty() {
            continue;
        }
        let cue = Cue {
            start,
            end,
            text: body.join("\n"),
        };
        if seen.insert(cue.clone()) {
            cues.push(cue);
        }
    }
    cues
}

/// TTML time expressions: `hh:mm:ss.fff`, `hh:mm:ss:frames`, `12.5s`,
/// `1500ms`, `2m`, `1h`, `900000t`.
fn ttml_time(value: &str, tick_rate: f64, frame_rate: f64) -> Option<u64> {
    let v = value.trim();
    if v.contains(':') {
        let parts: Vec<&str> = v.split(':').collect();
        if parts.len() < 3 {
            return None;
        }
        let h: f64 = parts[0].parse().ok()?;
        let m: f64 = parts[1].parse().ok()?;
        let mut s: f64 = parts[2].parse().ok()?;
        if let Some(frames) = parts.get(3) {
            s += frames.parse::<f64>().ok()? / frame_rate;
        }
        return Some(((h * 3600.0 + m * 60.0 + s) * 1000.0).round() as u64);
    }
    let units: [(&str, f64); 6] = [
        ("ms", 1.0),
        ("h", 3_600_000.0),
        ("m", 60_000.0),
        ("s", 1000.0),
        ("f", 1000.0 / frame_rate),
        ("t", 1000.0 / tick_rate),
    ];
    let (number, scale) = units
        .iter()
        .find_map(|(suffix, scale)| v.strip_suffix(suffix).map(|n| (n, *scale)))?;
    Some((number.parse::<f64>().ok()? * scale).round() as u64)
}

fn collect_text(node: roxmltree::Node, out: &mut String) {
    for child in node.children() {
        if child.is_text() {
            // XML whitespace between words collapses to one space.
            let text = child.text().unwrap_or("");
            let mut last_space = out.ends_with(' ') || out.ends_with('\n') || out.is_empty();
            for c in text.chars() {
                if c.is_whitespace() {
                    if !last_space {
                        out.push(' ');
                        last_space = true;
                    }
                } else {
                    out.push(c);
                    last_space = false;
                }
            }
        } else if child.is_element() {
            if child.tag_name().name() == "br" {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push('\n');
            } else {
                collect_text(child, out);
            }
        }
    }
}

pub fn parse_ttml(text: &str) -> Result<Vec<Cue>, MediaError> {
    let text = text.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(text).map_err(|e| MediaError::Xml(e.to_string()))?;
    let root = doc.root_element();
    let attr = |name: &str| {
        root.attributes()
            .find(|a| a.name() == name)
            .map(|a| a.value().to_string())
    };
    let tick_rate: f64 = attr("tickRate")
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000_000.0);
    let frame_rate: f64 = attr("frameRate")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    let mut cues = Vec::new();
    let mut seen: HashSet<Cue> = HashSet::new();
    for p in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "p")
    {
        let get = |name: &str| p.attributes().find(|a| a.name() == name).map(|a| a.value());
        let Some(begin) = get("begin").and_then(|v| ttml_time(v, tick_rate, frame_rate)) else {
            continue;
        };
        let end = get("end")
            .and_then(|v| ttml_time(v, tick_rate, frame_rate))
            .or_else(|| {
                get("dur")
                    .and_then(|v| ttml_time(v, tick_rate, frame_rate))
                    .map(|d| begin + d)
            });
        let Some(end) = end else { continue };
        let mut body = String::new();
        collect_text(p, &mut body);
        let body = body.trim().to_string();
        if body.is_empty() {
            continue;
        }
        let cue = Cue {
            start: begin,
            end,
            text: body,
        };
        if seen.insert(cue.clone()) {
            cues.push(cue);
        }
    }
    Ok(cues)
}
