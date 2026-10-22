//! Mohona media.
//!
//! Pure parsing, no network: this crate turns the text of a streaming
//! manifest into the common model in [`model`] (variants, tracks, segments).
//! Fetching the manifest and its segments is done by the callers, with the
//! normal Mohona transports.

pub mod dash;
mod error;
pub mod hls;
mod model;
pub mod mux;
pub mod quality;

pub use error::MediaError;
pub use model::{
    ByteRange, Encryption, InitSegment, MediaKind, MediaVariant, Rendition, Resolution, Segment,
    SegmentList, Track,
};
