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

    /// A byte range was requested but the server answered with something else,
    /// for example the whole file (status 200) or a different part.
    /// This also happens when the file changed on the server.
    #[error("the server did not send the requested part of the file ({0})")]
    WrongRange(String),

    /// The request failed before or while receiving the response.
    #[error("network error")]
    Http(#[from] reqwest::Error),
}
