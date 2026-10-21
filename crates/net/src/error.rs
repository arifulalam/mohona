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
    #[error(
        "unsupported URL scheme \"{0}\" (only http, https, webdav, ftp, ftps, ftpes and sftp work so far)"
    )]
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

    /// An FTP server answered with an error.
    #[error("{}", explain_ftp(*.code, .message))]
    Ftp {
        /// The three-digit reply code.
        code: u16,
        /// What the server said.
        message: String,
    },

    /// A connection could not be made or was lost (FTP, SFTP).
    #[error("connection problem: {0}")]
    Connection(std::io::Error),

    /// An SSH problem other than the login or the server's key.
    #[error("SSH problem: {0}")]
    Ssh(String),

    /// The SSH login did not work.
    #[error("SFTP login: {0}")]
    SshLogin(String),

    /// The server's SSH key was not accepted.
    #[error("the server's SSH key was not accepted: {0}")]
    HostKey(String),

    /// An SFTP error.
    #[error("SFTP: {message}")]
    Sftp {
        /// What went wrong.
        message: String,
        /// Trying again cannot help.
        fatal: bool,
    },

    /// A checksum cannot be read.
    #[error("invalid checksum {0}")]
    InvalidChecksum(String),

    /// The server's certificate was not accepted.
    #[error("the server's certificate cannot be trusted: {0}. If you trust this server, give the authority that signed it with --ca-file FILE, or skip the check for this download with --insecure")]
    Tls(String),

    /// The file given as a certificate authority cannot be used.
    #[error("cannot use the certificate file {path}: {reason}")]
    InvalidCaFile {
        /// The file.
        path: String,
        /// What is wrong with it.
        reason: String,
    },

    /// The request failed before or while receiving the response.
    #[error("network error")]
    Http(#[source] reqwest::Error),
}

impl From<reqwest::Error> for NetError {
    fn from(error: reqwest::Error) -> Self {
        match explain_certificate(&error) {
            Some(reason) => NetError::Tls(reason),
            None => NetError::Http(error),
        }
    }
}

/// If the error is about the server's certificate, say what is wrong in
/// plain words.
fn explain_certificate(error: &reqwest::Error) -> Option<String> {
    use std::error::Error;
    // The debug form names the exact certificate problem even where the
    // readable message does not.
    let mut text = format!("{error:?} ");
    let mut source: Option<&dyn Error> = Some(error);
    while let Some(e) = source {
        text.push_str(&e.to_string());
        text.push(' ');
        source = e.source();
    }
    certificate_problem(&text)
}

/// If the text (an error and its causes) is about a server certificate, say
/// what is wrong in plain words.
pub(crate) fn certificate_problem(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if !(lower.contains("certificate") || lower.contains("unknownissuer")) {
        return None;
    }
    let reason = if lower.contains("unknownissuer") || lower.contains("unknown issuer") {
        "it was not signed by an authority this computer trusts"
    } else if lower.contains("expired") {
        "it has expired"
    } else if lower.contains("notvalidyet") || lower.contains("not valid yet") {
        "it is not valid yet (is this computer's clock right?)"
    } else if lower.contains("notvalidforname") || lower.contains("not valid for name") {
        "it is for a different name than the one in the address"
    } else {
        "it was refused"
    };
    Some(reason.to_string())
}

/// Say what an FTP error code means in this case, and what to do about it.
fn explain_ftp(code: u16, message: &str) -> String {
    let hint = match code {
        421 => " (the server is busy or limits connections: try fewer with -c)",
        530 | 331 | 332 => " (it refused the login: give one with -u user:password)",
        550 => " (the file is not there, or you may not read it)",
        _ => "",
    };
    format!("the FTP server answered {code}: {message}{hint}")
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
            NetError::Ftp { code, .. } => match code {
                421 => Failure::Throttled,
                425 | 426 | 450 | 451 | 452 => Failure::Transient,
                _ => Failure::Fatal,
            },
            NetError::Connection(error) => match error.kind() {
                std::io::ErrorKind::ConnectionRefused => Failure::Throttled,
                _ => Failure::Transient,
            },
            NetError::Ssh(_) => Failure::Transient,
            NetError::Sftp { fatal, .. } => {
                if *fatal {
                    Failure::Fatal
                } else {
                    Failure::Transient
                }
            }
            NetError::SshLogin(_) | NetError::HostKey(_) => Failure::Fatal,
            NetError::InvalidUrl(_)
            | NetError::Unauthorized { .. }
            | NetError::UnsupportedScheme(_)
            | NetError::InvalidHeader(_)
            | NetError::InvalidProxy(_)
            | NetError::Tls(_)
            | NetError::InvalidChecksum(_)
            | NetError::InvalidCaFile { .. }
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
