//! MPEG-DASH manifests (MPD), ISO/IEC 23009-1.
//!
//! [`parse`] lists every stream of a *static* manifest as a [`Rendition`] with
//! all its segments, following `SegmentTemplate` (by number, by time, or with
//! a `SegmentTimeline`), `SegmentList` and `SegmentBase`, across periods.
//! Streams with `ContentProtection` are refused as DRM. Live (`dynamic`)
//! manifests are not supported yet.

use roxmltree::{Document, Node};
use url::Url;

use crate::error::MediaError;
use crate::model::{
    ByteRange, InitSegment, MediaKind, Rendition, Resolution, Segment, SegmentList,
};

fn invalid(message: impl Into<String>) -> MediaError {
    MediaError::Invalid(message.into())
}

/// Reads a manifest fetched from `base`.
pub fn parse(text: &str, base: &Url) -> Result<Vec<Rendition>, MediaError> {
    let text = text.trim_start_matches('\u{feff}');
    let doc = Document::parse(text).map_err(|e| MediaError::Xml(e.to_string()))?;
    let mpd = doc.root_element();
    if mpd.tag_name().name() != "MPD" {
        return Err(invalid("the XML is not a DASH manifest (no MPD element)"));
    }
    if mpd.attribute("type") == Some("dynamic") {
        return Err(invalid(
            "this is a live DASH stream; live streams are not supported yet",
        ));
    }
    let total = mpd
        .attribute("mediaPresentationDuration")
        .map(duration)
        .transpose()?;
    let mpd_base = with_base(base, mpd)?;

    let periods: Vec<Node> = children(mpd, "Period").collect();
    let mut renditions: Vec<Rendition> = Vec::new();
    let mut start = 0.0;
    for (index, period) in periods.iter().enumerate() {
        if let Some(s) = period.attribute("start") {
            start = duration(s)?;
        }
        let length = match period.attribute("duration") {
            Some(d) => Some(duration(d)?),
            None => match periods.get(index + 1).and_then(|p| p.attribute("start")) {
                Some(next) => Some(duration(next)? - start),
                None => total.map(|t| t - start),
            },
        };
        let period_base = with_base(&mpd_base, *period)?;
        for set in children(*period, "AdaptationSet") {
            let set_base = with_base(&period_base, set)?;
            for rep in children(set, "Representation") {
                let rep_base = with_base(&set_base, rep)?;
                let one = representation(*period, set, rep, &rep_base, length, index > 0)?;
                match renditions
                    .iter_mut()
                    .find(|r| r.kind == one.kind && r.id == one.id)
                {
                    Some(existing) => {
                        existing.segments.segments.extend(one.segments.segments);
                    }
                    None => renditions.push(one),
                }
            }
        }
        start += length.unwrap_or(0.0);
    }
    if renditions.is_empty() {
        return Err(MediaError::Empty);
    }
    // Sequence numbers run on across periods.
    for r in &mut renditions {
        for (i, s) in r.segments.segments.iter_mut().enumerate() {
            s.sequence = i as u64;
        }
    }
    Ok(renditions)
}

fn children<'a, 'i: 'a>(
    node: Node<'a, 'i>,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn first<'a, 'i: 'a>(node: Node<'a, 'i>, name: &'static str) -> Option<Node<'a, 'i>> {
    children(node, name).next()
}

/// The address a node's `BaseURL` makes, relative to its parent's.
fn with_base(parent: &Url, node: Node) -> Result<Url, MediaError> {
    match first(node, "BaseURL").and_then(|b| b.text()) {
        Some(text) => parent
            .join(text.trim())
            .map_err(|e| invalid(format!("bad BaseURL \"{}\": {e}", text.trim()))),
        None => Ok(parent.clone()),
    }
}

/// ISO 8601 durations such as `PT1H2M3.5S` or `P1DT2H`, in seconds.
fn duration(value: &str) -> Result<f64, MediaError> {
    let bad = || invalid(format!("bad duration \"{value}\""));
    let rest = value.trim().strip_prefix('P').ok_or_else(bad)?;
    let (date, time) = match rest.split_once('T') {
        Some((d, t)) => (d, t),
        None => (rest, ""),
    };
    let mut seconds = 0.0;
    for (part, units) in [
        (date, [('D', 86_400.0)].as_slice()),
        (time, [('H', 3600.0), ('M', 60.0), ('S', 1.0)].as_slice()),
    ] {
        let mut number = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() || c == '.' {
                number.push(c);
            } else {
                let unit = units.iter().find(|(u, _)| *u == c).ok_or_else(bad)?;
                seconds += number.parse::<f64>().map_err(|_| bad())? * unit.1;
                number.clear();
            }
        }
        if !number.is_empty() {
            return Err(bad());
        }
    }
    Ok(seconds)
}

/// The nearest attribute going from the representation outwards.
fn inherited<'a, 'i: 'a>(nodes: &[Node<'a, 'i>], attribute: &str) -> Option<&'a str> {
    nodes.iter().find_map(|n| n.attribute(attribute))
}

fn kind_of(set: Node, rep: Node) -> MediaKind {
    let content = set.attribute("contentType");
    let mime = rep
        .attribute("mimeType")
        .or_else(|| set.attribute("mimeType"))
        .unwrap_or("");
    let text = |s: &str| {
        s == "text" || s.starts_with("text/") || mime.contains("ttml") || mime.contains("vtt")
    };
    if content == Some("audio") || mime.starts_with("audio/") {
        MediaKind::Audio
    } else if content.is_some_and(text) || text(mime) || mime.starts_with("application/ttml") {
        MediaKind::Subtitles
    } else {
        MediaKind::Video
    }
}

fn representation(
    period: Node,
    set: Node,
    rep: Node,
    base: &Url,
    length: Option<f64>,
    after_first_period: bool,
) -> Result<Rendition, MediaError> {
    // DRM: any ContentProtection on the way down means encrypted media.
    for node in [period, set, rep] {
        if let Some(p) = first(node, "ContentProtection") {
            let scheme = p.attribute("schemeIdUri").unwrap_or("unknown");
            return Err(MediaError::Protected(format!(
                "content protection {scheme}"
            )));
        }
    }
    let nodes = [rep, set];
    let id = rep.attribute("id").unwrap_or("").to_string();
    let bandwidth: u64 = rep
        .attribute("bandwidth")
        .and_then(|b| b.parse().ok())
        .unwrap_or(0);
    let width = inherited(&nodes, "width").and_then(|v| v.parse().ok());
    let height = inherited(&nodes, "height").and_then(|v| v.parse().ok());
    let list = segments_of(period, set, rep, base, &id, bandwidth, length)?;
    let mut list = list;
    if after_first_period {
        if let Some(first) = list.first_mut() {
            first.discontinuity = true;
        }
    }
    let target = list.iter().map(|s| s.duration).fold(0.0, f64::max);
    Ok(Rendition {
        kind: kind_of(set, rep),
        id,
        bandwidth,
        resolution: match (width, height) {
            (Some(width), Some(height)) => Some(Resolution { width, height }),
            _ => None,
        },
        codecs: inherited(&nodes, "codecs").map(str::to_string),
        language: set.attribute("lang").map(str::to_string),
        mime_type: inherited(&nodes, "mimeType").map(str::to_string),
        segments: SegmentList {
            segments: list,
            ended: true,
            target_duration: target,
        },
    })
}

fn join(base: &Url, reference: &str) -> Result<Url, MediaError> {
    base.join(reference)
        .map_err(|e| invalid(format!("bad address \"{reference}\": {e}")))
}

/// `first-last` as used by DASH byte ranges.
fn range(value: &str) -> Result<ByteRange, MediaError> {
    let bad = || invalid(format!("bad byte range \"{value}\""));
    let (a, b) = value.split_once('-').ok_or_else(bad)?;
    let (a, b): (u64, u64) = (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?);
    if b < a {
        return Err(bad());
    }
    Ok(ByteRange {
        offset: a,
        length: b - a + 1,
    })
}

fn find_in<'a, 'i: 'a>(name: &'static str, nodes: [Node<'a, 'i>; 3]) -> Option<Node<'a, 'i>> {
    nodes.iter().find_map(|n| first(*n, name))
}

fn segments_of(
    period: Node,
    set: Node,
    rep: Node,
    base: &Url,
    id: &str,
    bandwidth: u64,
    length: Option<f64>,
) -> Result<Vec<Segment>, MediaError> {
    let nodes = [rep, set, period];
    let make =
        |url: Url, range: Option<ByteRange>, duration: f64, init: &Option<InitSegment>| Segment {
            sequence: 0,
            url,
            range,
            duration,
            encryption: None,
            init: init.clone(),
            discontinuity: false,
        };

    if let Some(template) = find_in("SegmentTemplate", nodes) {
        let timescale: f64 = template
            .attribute("timescale")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        let start_number: u64 = template
            .attribute("startNumber")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let init = match template.attribute("initialization") {
            Some(t) => Some(InitSegment {
                url: join(base, &expand(t, id, bandwidth, start_number, 0))?,
                range: None,
            }),
            None => None,
        };
        let media = template
            .attribute("media")
            .ok_or_else(|| invalid("SegmentTemplate without media"))?;
        let mut out = Vec::new();
        if let Some(timeline) = first(template, "SegmentTimeline") {
            let mut time: u64 = 0;
            let mut number = start_number;
            for s in children(timeline, "S") {
                if let Some(t) = s.attribute("t") {
                    time = t.parse().map_err(|_| invalid("bad S@t"))?;
                }
                let d: u64 = s
                    .attribute("d")
                    .and_then(|v| v.parse().ok())
                    .filter(|d| *d > 0)
                    .ok_or_else(|| invalid("S without a usable d"))?;
                let r: i64 = s.attribute("r").and_then(|v| v.parse().ok()).unwrap_or(0);
                let repeats = if r >= 0 {
                    r as u64 + 1
                } else {
                    // Repeat until the period ends.
                    let end = length
                        .ok_or_else(|| invalid("S@r=-1 needs a known period length"))?
                        * timescale;
                    (((end - time as f64) / d as f64).ceil().max(1.0)) as u64
                };
                for _ in 0..repeats {
                    out.push(make(
                        join(base, &expand(media, id, bandwidth, number, time))?,
                        None,
                        d as f64 / timescale,
                        &init,
                    ));
                    time += d;
                    number += 1;
                }
            }
        } else {
            let d: f64 = template
                .attribute("duration")
                .and_then(|v| v.parse().ok())
                .filter(|d: &f64| *d > 0.0)
                .ok_or_else(|| invalid("SegmentTemplate without duration or timeline"))?;
            let seconds = d / timescale;
            let length = length.ok_or_else(|| {
                invalid("the period length is not known (no mediaPresentationDuration)")
            })?;
            let count = (length / seconds).ceil().max(1.0) as u64;
            for i in 0..count {
                let number = start_number + i;
                let time = i * d as u64;
                out.push(make(
                    join(base, &expand(media, id, bandwidth, number, time))?,
                    None,
                    seconds,
                    &init,
                ));
            }
        }
        return finish(out);
    }

    if let Some(list) = find_in("SegmentList", nodes) {
        let timescale: f64 = list
            .attribute("timescale")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        let seconds = list
            .attribute("duration")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|d| d / timescale)
            .unwrap_or(0.0);
        let init = match first(list, "Initialization") {
            Some(i) => Some(InitSegment {
                url: match i.attribute("sourceURL") {
                    Some(u) => join(base, u)?,
                    None => base.clone(),
                },
                range: i.attribute("range").map(range).transpose()?,
            }),
            None => None,
        };
        let mut out = Vec::new();
        for s in children(list, "SegmentURL") {
            let url = match s.attribute("media") {
                Some(m) => join(base, m)?,
                None => base.clone(),
            };
            out.push(make(
                url,
                s.attribute("mediaRange").map(range).transpose()?,
                seconds,
                &init,
            ));
        }
        return finish(out);
    }

    if find_in("SegmentBase", nodes).is_some() || first(rep, "BaseURL").is_some() {
        // One whole file (on-demand profile): the init part is inside it.
        return finish(vec![make(base.clone(), None, length.unwrap_or(0.0), &None)]);
    }
    Err(invalid(format!(
        "representation \"{id}\" says nothing about where its segments are"
    )))
}

fn finish(segments: Vec<Segment>) -> Result<Vec<Segment>, MediaError> {
    if segments.is_empty() {
        Err(MediaError::Empty)
    } else {
        Ok(segments)
    }
}

/// Fills `$RepresentationID$`, `$Number$`, `$Bandwidth$` and `$Time$`
/// (with optional `%0Nd` padding) in a template.
fn expand(template: &str, id: &str, bandwidth: u64, number: u64, time: u64) -> String {
    let mut out = String::new();
    let mut parts = template.split('$');
    out.push_str(parts.next().unwrap_or(""));
    let mut inside = true;
    for part in parts {
        if inside {
            if part.is_empty() {
                out.push('$');
            } else {
                let (name, format) = match part.split_once('%') {
                    Some((n, f)) => (n, Some(f)),
                    None => (part, None),
                };
                let value = match name {
                    "RepresentationID" => {
                        out.push_str(id);
                        None
                    }
                    "Number" => Some(number),
                    "Bandwidth" => Some(bandwidth),
                    "Time" => Some(time),
                    _ => {
                        out.push('$');
                        out.push_str(part);
                        out.push('$');
                        None
                    }
                };
                if let Some(value) = value {
                    let width = format
                        .and_then(|f| f.strip_prefix('0'))
                        .and_then(|f| f.strip_suffix('d'))
                        .and_then(|f| f.parse::<usize>().ok())
                        .unwrap_or(0);
                    out.push_str(&format!("{value:0width$}"));
                }
            }
        } else {
            out.push_str(part);
        }
        inside = !inside;
    }
    out
}
