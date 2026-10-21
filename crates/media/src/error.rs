/// Why a manifest could not be used.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MediaError {
    /// The text is not a playlist at all (HLS playlists start with `#EXTM3U`).
    #[error("this is not an HLS playlist (it does not start with #EXTM3U)")]
    NotAPlaylist,
    /// A tag or attribute is malformed.
    #[error("the playlist is damaged: {0}")]
    Invalid(String),
    /// The stream is protected in a way Mohona does not handle (DRM).
    #[error("this stream is protected ({0}); DRM-protected streams are not supported")]
    Protected(String),
    /// The manifest is not valid XML.
    #[error("the manifest is not readable XML: {0}")]
    Xml(String),
    /// The playlist has no segments to download.
    #[error("the playlist lists no segments")]
    Empty,
}
