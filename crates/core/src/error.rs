use mohona_net::NetError;
use thiserror::Error;

/// Errors from the download engine.
#[derive(Debug, Error)]
pub enum Error {
    /// The network layer failed (bad URL, connection problem, server error).
    #[error(transparent)]
    Net(#[from] NetError),

    /// Reading or writing a file failed.
    #[error("could not write the file")]
    Io(#[from] std::io::Error),

    /// The download was stopped on request. What was downloaded is kept, and
    /// running the same download again continues from there.
    #[error("the download was stopped")]
    Cancelled,

    /// The finished file does not hash to what it should. The file is kept.
    #[error("the file does not match its checksum: expected {algorithm}:{expected} but it is {algorithm}:{actual}. The file was kept at {path} (it may be damaged, or the checksum may be wrong)", path = path.display())]
    ChecksumMismatch {
        /// The algorithm used.
        algorithm: String,
        /// What it should have been.
        expected: String,
        /// What it is.
        actual: String,
        /// Where the file was kept.
        path: std::path::PathBuf,
    },

    /// The transfer ended without an error, but with fewer bytes than promised.
    #[error("the download ended early: received {received} of {expected} bytes")]
    Incomplete {
        /// Size the server promised.
        expected: u64,
        /// Bytes actually received.
        received: u64,
    },
}

impl Error {
    /// Whether a partial file left by this failure is useless (the file
    /// changed on the server), as opposed to something that can be resumed.
    pub(crate) fn makes_partial_file_useless(&self) -> bool {
        matches!(self, Error::Net(NetError::WrongRange(_)))
    }
}
