use std::time::Duration;

use thiserror::Error;

/// How a failed request should be handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Trying again cannot help (bad URL, file changed, 404, ...).
    Fatal,
    /// A hiccup: trying again after a short wait may work.
    Transient,
    /// The server is limiting us (429, 503, refused or dropped new
    /// connections): wait longer, and use fewer connections.
    Throttled,
}

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
        /// How long the server asked us to wait (`Retry-After`), if it did.
        retry_after: Option<Duration>,
    },

    /// The server wants a login (`401`).
    #[error("{}", explain_unauthorized(.url, .offered, *.credentials_given))]
    Unauthorized {
        /// The URL that was requested.
        url: String,
        /// The kinds of login the server named (`Basic`, `Bearer`, ...).
        offered: Vec<String>,
        /// Whether a user name and password were given.
        credentials_given: bool,
    },

    /// A byte range was requested but the server answered with something else,
    /// for example the whole file (status 200) or a different part.
    /// This also happens when the file changed on the server.
    #[error("the server did not send the requested part of the file ({0})")]
    WrongRange(String),

    /// A proxy address cannot be used.
    #[error("invalid proxy: {0}")]
    InvalidProxy(String),

    /// The proxy wants a login (`407`).
    #[error("the proxy wants a login (407): put it in the proxy address, like http://user:password@host:port")]
    ProxyAuthRequired,

    /// A header, cookie or the like cannot be sent.
    #[error("invalid header: {0}")]
    InvalidHeader(String),

    /// The request failed before or while receiving the response.
    #[error("network error")]
    Http(#[from] reqwest::Error),
}

/// Say what a `401` means in this case, and what to do about it.
fn explain_unauthorized(url: &str, offered: &[String], credentials_given: bool) -> String {
    let usable =
        |name: &&String| name.eq_ignore_ascii_case("basic") || name.eq_ignore_ascii_case("digest");
    let has_usable = offered.iter().any(|n| usable(&n));
    let names = offered.join(", ");
    let reason = if offered.is_empty() {
        "it did not say how to log in. The site may use a login page and cookies instead: \
         log in with your browser and pass its cookie with --cookie"
            .to_string()
    } else if !has_usable {
        format!(
            "it asks for {names}, which Mohona does not support (it can do Basic and Digest). \
             For a token, send it as a header with -H \"Authorization: ...\""
        )
    } else if credentials_given {
        format!("it refused the user name or password (it accepts {names})")
    } else {
        format!("it asks for a login ({names}): give one with -u user:password")
    };
    format!("the server answered 401 for {url}: {reason}")
}

impl NetError {
    /// Whether this failure is worth retrying, and how.
    pub fn failure(&self) -> Failure {
        match self {
            NetError::Status { status, .. } => match status {
                429 | 503 => Failure::Throttled,
                408 | 500 | 502 | 504 => Failure::Transient,
                _ => Failure::Fatal,
            },
            NetError::Http(error) => {
                if error.is_connect() {
                    Failure::Throttled
                } else if error.is_builder() || error.is_redirect() {
                    Failure::Fatal
                } else {
                    Failure::Transient
                }
            }
            NetError::InvalidUrl(_)
            | NetError::Unauthorized { .. }
            | NetError::UnsupportedScheme(_)
            | NetError::InvalidHeader(_)
            | NetError::InvalidProxy(_)
            | NetError::ProxyAuthRequired
            | NetError::WrongRange(_) => Failure::Fatal,
        }
    }

    /// The wait the server asked for, if any.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            NetError::Status { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}
