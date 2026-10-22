//! Choosing a quality, an audio language and so on.

use crate::model::{MediaKind, MediaVariant, Rendition, Track};

/// What the person asked for with `--quality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// The highest bit rate (the default).
    Best,
    /// The lowest bit rate that still has video.
    Worst,
    /// The best one that is not taller than this many pixels; if every one
    /// is taller, the smallest of them.
    Height(u32),
    /// Sound only, the best bit rate.
    AudioOnly,
}

impl Quality {
    /// `best`, `worst`, `audio`, or a height like `720` or `720p`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim().to_ascii_lowercase();
        match t.as_str() {
            "best" => Ok(Quality::Best),
            "worst" => Ok(Quality::Worst),
            "audio" | "audio-only" => Ok(Quality::AudioOnly),
            _ => t
                .strip_suffix('p')
                .unwrap_or(&t)
                .parse::<u32>()
                .ok()
                .filter(|h| *h > 0)
                .map(Quality::Height)
                .ok_or_else(|| {
                    format!("\"{text}\" is not a quality (use best, worst, audio, or a height like 720)")
                }),
        }
    }
}

/// Anything that has a bit rate and maybe a height.
trait Candidate {
    fn bandwidth(&self) -> u64;
    fn height(&self) -> Option<u32>;
    fn has_video(&self) -> bool;
}

impl Candidate for MediaVariant {
    fn bandwidth(&self) -> u64 {
        self.bandwidth
    }
    fn height(&self) -> Option<u32> {
        self.resolution.map(|r| r.height)
    }
    fn has_video(&self) -> bool {
        !self.is_audio_only()
    }
}

impl Candidate for Rendition {
    fn bandwidth(&self) -> u64 {
        self.bandwidth
    }
    fn height(&self) -> Option<u32> {
        self.resolution.map(|r| r.height)
    }
    fn has_video(&self) -> bool {
        self.kind == MediaKind::Video
    }
}

fn choose<T: Candidate>(items: &[T], quality: Quality) -> Option<usize> {
    let indices = |video: bool| (0..items.len()).filter(move |i| items[*i].has_video() == video);
    let by_rate = |a: &usize, b: &usize| items[*a].bandwidth().cmp(&items[*b].bandwidth());
    // Video first; a stream that is only sound still has a "best".
    let pool: Vec<usize> = match quality {
        Quality::AudioOnly => indices(false).collect(),
        _ => {
            let video: Vec<usize> = indices(true).collect();
            if video.is_empty() {
                indices(false).collect()
            } else {
                video
            }
        }
    };
    match quality {
        Quality::Best | Quality::AudioOnly => pool.into_iter().max_by(by_rate),
        Quality::Worst => pool.into_iter().min_by(by_rate),
        Quality::Height(limit) => {
            let fitting = pool
                .iter()
                .copied()
                .filter(|i| items[*i].height().is_some_and(|h| h <= limit))
                .max_by(|a, b| {
                    items[*a]
                        .height()
                        .cmp(&items[*b].height())
                        .then(items[*a].bandwidth().cmp(&items[*b].bandwidth()))
                });
            fitting.or_else(|| {
                pool.into_iter().min_by(|a, b| {
                    items[*a]
                        .height()
                        .cmp(&items[*b].height())
                        .then(items[*a].bandwidth().cmp(&items[*b].bandwidth()))
                })
            })
        }
    }
}

/// The index of the HLS variant that best fits `quality`.
pub fn pick_variant(variants: &[MediaVariant], quality: Quality) -> Option<usize> {
    choose(variants, quality)
}

/// The index of the DASH video (or, for [`Quality::AudioOnly`], audio)
/// rendition that best fits `quality`.
pub fn pick_rendition(renditions: &[Rendition], quality: Quality) -> Option<usize> {
    let wanted = if quality == Quality::AudioOnly {
        MediaKind::Audio
    } else {
        MediaKind::Video
    };
    let of_kind: Vec<usize> = (0..renditions.len())
        .filter(|i| renditions[*i].kind == wanted)
        .collect();
    let subset: Vec<Rendition> = of_kind.iter().map(|i| renditions[*i].clone()).collect();
    let pick = if wanted == MediaKind::Audio {
        choose(&subset, Quality::AudioOnly)
    } else {
        choose(&subset, quality)
    };
    pick.map(|i| of_kind[i])
}

fn language_matches(have: &str, want: &str) -> bool {
    let (have, want) = (have.to_ascii_lowercase(), want.to_ascii_lowercase());
    have == want || have.split('-').next() == want.split('-').next()
}

/// The audio track to download next to an HLS variant: one in the variant's
/// group that has its own playlist, preferring the language asked for, then
/// the default one.
pub fn pick_track<'a>(
    tracks: &'a [Track],
    kind: MediaKind,
    group: &str,
    language: Option<&str>,
) -> Option<&'a Track> {
    let of_group: Vec<&Track> = tracks
        .iter()
        .filter(|t| t.kind == kind && t.group == group && t.url.is_some())
        .collect();
    if let Some(want) = language {
        let found = of_group.iter().find(|t| {
            t.language
                .as_deref()
                .is_some_and(|l| language_matches(l, want))
                || t.name.eq_ignore_ascii_case(want)
        });
        return found.copied();
    }
    of_group
        .iter()
        .find(|t| t.default)
        .or_else(|| of_group.first())
        .copied()
}

/// The audio rendition of a DASH manifest to go with its video: the language
/// asked for, else the first language listed, at the best bit rate.
pub fn pick_audio_rendition(renditions: &[Rendition], language: Option<&str>) -> Option<usize> {
    let audio: Vec<usize> = (0..renditions.len())
        .filter(|i| renditions[*i].kind == MediaKind::Audio)
        .collect();
    let wanted: Option<&str> = match language {
        Some(l) => Some(l),
        None => audio
            .first()
            .and_then(|i| renditions[*i].language.as_deref()),
    };
    let matching: Vec<usize> = audio
        .iter()
        .copied()
        .filter(|i| match (wanted, renditions[*i].language.as_deref()) {
            (Some(w), Some(l)) => language_matches(l, w),
            (None, _) => true,
            (Some(_), None) => false,
        })
        .collect();
    let pool = if matching.is_empty() && language.is_none() {
        audio
    } else {
        matching
    };
    pool.into_iter().max_by_key(|i| renditions[*i].bandwidth)
}

/// The DASH subtitle renditions, optionally for one language.
pub fn pick_subtitles<'a>(
    renditions: &'a [Rendition],
    language: Option<&str>,
) -> Vec<&'a Rendition> {
    renditions
        .iter()
        .filter(|r| r.kind == MediaKind::Subtitles)
        .filter(|r| match (language, r.language.as_deref()) {
            (Some(w), Some(l)) => language_matches(l, w),
            (Some(_), None) => false,
            (None, _) => true,
        })
        .collect()
}
