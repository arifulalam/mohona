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

    /// The transfer ended without an error, but with fewer bytes than promised.
    #[error("the download ended early: received {received} of {expected} bytes")]
    Incomplete {
        /// Size the server promised.
        expected: u64,
        /// Bytes actually received.
        received: u64,
    },
}
