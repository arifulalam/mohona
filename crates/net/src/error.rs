use thiserror::Error;

/// Errors from the network layer.
#[derive(Debug, Error)]
pub enum NetError {
    /// The text is not a valid URL.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    /// The URL uses a scheme Mohona cannot download yet.
    #[error("unsupported URL scheme \"{0}\" (only http and https work so far)")]
    UnsupportedScheme(String),

    /// The server answered, but not with a success status.
    #[error("the server answered {status} for {url}")]
    Status {
        /// HTTP status code.
        status: u16,
        /// The URL that was requested.
        url: String,
    },

    /// The request failed before or while receiving the response.
    #[error("network error")]
    Http(#[from] reqwest::Error),
}
