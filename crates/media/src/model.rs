use serde::{Deserialize, Serialize};
use url::Url;

/// What a track or variant carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Video,
    Audio,
    Subtitles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

/// One quality of a stream, as offered by a master playlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaVariant {
    /// Playlist with this variant's segments.
    pub url: Url,
    /// Peak bits per second, as announced.
    pub bandwidth: u64,
    pub average_bandwidth: Option<u64>,
    pub resolution: Option<Resolution>,
    pub frame_rate: Option<f64>,
    /// Codec list as announced, e.g. `avc1.4d401f,mp4a.40.2`.
    pub codecs: Option<String>,
    /// Name of the audio group this variant plays with, if audio is separate.
    pub audio_group: Option<String>,
    pub subtitle_group: Option<String>,
}

impl MediaVariant {
    /// A short label for lists, e.g. `720p 2.5 Mbit/s`.
    pub fn label(&self) -> String {
        let rate = self.bandwidth as f64 / 1_000_000.0;
        match self.resolution {
            Some(r) => format!("{}p {rate:.1} Mbit/s", r.height),
            None => format!("{rate:.1} Mbit/s"),
        }
    }

    /// True when the codec list names no video codec (an audio-only variant).
    pub fn is_audio_only(&self) -> bool {
        match &self.codecs {
            Some(codecs) => {
                self.resolution.is_none()
                    && !codecs.split(',').any(|c| {
                        let c = c.trim();
                        c.starts_with("avc")
                            || c.starts_with("hvc")
                            || c.starts_with("hev")
                            || c.starts_with("vp0")
                            || c.starts_with("av01")
                    })
            }
            None => false,
        }
    }
}

/// A separate audio or subtitle rendition (`#EXT-X-MEDIA`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub kind: MediaKind,
    pub group: String,
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
    /// Absent when the audio is already inside the variant's own segments.
    pub url: Option<Url>,
}

/// A part of a file: `length` bytes from `offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

/// How segments are encrypted. DRM never appears here: it is refused earlier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encryption {
    /// AES-128 in CBC mode; the key is fetched from `key_url`.
    Aes128 {
        key_url: Url,
        /// Explicit initialisation vector, else derived from the segment number.
        iv: Option<[u8; 16]>,
    },
}

/// The header that fragmented MP4 segments need in front of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitSegment {
    pub url: Url,
    pub range: Option<ByteRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Position in the whole stream (`media sequence` + index); used for IVs.
    pub sequence: u64,
    pub url: Url,
    pub range: Option<ByteRange>,
    pub duration: f64,
    pub encryption: Option<Encryption>,
    pub init: Option<InitSegment>,
    /// A change of format or timeline starts here.
    pub discontinuity: bool,
}

impl Segment {
    /// The 16-byte IV for this segment: the playlist's, or its number.
    pub fn iv(&self) -> Option<[u8; 16]> {
        match &self.encryption {
            Some(Encryption::Aes128 { iv: Some(iv), .. }) => Some(*iv),
            Some(Encryption::Aes128 { iv: None, .. }) => {
                let mut iv = [0u8; 16];
                iv[8..].copy_from_slice(&self.sequence.to_be_bytes());
                Some(iv)
            }
            None => None,
        }
    }
}

/// Everything needed to download one rendition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentList {
    pub segments: Vec<Segment>,
    /// False for a live playlist that may still grow.
    pub ended: bool,
    pub target_duration: f64,
}

impl SegmentList {
    pub fn total_duration(&self) -> f64 {
        self.segments.iter().map(|s| s.duration).sum()
    }
}

/// One downloadable stream of a DASH manifest: a quality of video, an audio
/// language or a subtitle track, with its segments already listed (all
/// periods joined).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rendition {
    pub kind: MediaKind,
    pub id: String,
    pub bandwidth: u64,
    pub resolution: Option<Resolution>,
    pub codecs: Option<String>,
    pub language: Option<String>,
    pub mime_type: Option<String>,
    pub segments: SegmentList,
}
