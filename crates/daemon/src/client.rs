use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::info::DaemonInfo;
use crate::protocol::{Call, Request, Response};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// There is no info file, or the daemon it names does not answer.
    #[error("the Mohona daemon is not running (start it with `mohona daemon`)")]
    NotRunning,
    #[error("the daemon did not answer properly: {0}")]
    Protocol(String),
}

/// A client of the daemon. Each call uses its own short connection.
#[derive(Debug, Clone)]
pub struct Client {
    addr: SocketAddr,
    token: String,
}

const TIMEOUT: Duration = Duration::from_secs(30);

impl Client {
    pub fn new(info: &DaemonInfo) -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], info.port)),
            token: info.token.clone(),
        }
    }

    /// Connects using the info file the daemon wrote.
    pub fn from_info_file(path: &Path) -> Result<Self, ClientError> {
        DaemonInfo::read(path)
            .map(|info| Self::new(&info))
            .map_err(|_| ClientError::NotRunning)
    }

    pub async fn call(&self, call: Call) -> Result<Response, ClientError> {
        let line = serde_json::to_string(&Request {
            token: self.token.clone(),
            call,
        })
        .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let stream = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(self.addr))
            .await
            .map_err(|_| ClientError::NotRunning)?
            .map_err(|_| ClientError::NotRunning)?;
        let (read, mut write) = stream.into_split();
        write
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        let mut reader = BufReader::new(read);
        let mut answer = String::new();
        tokio::time::timeout(TIMEOUT, reader.read_line(&mut answer))
            .await
            .map_err(|_| ClientError::Protocol("no answer in time".into()))?
            .map_err(|e| ClientError::Protocol(e.to_string()))?;
        serde_json::from_str(&answer).map_err(|e| ClientError::Protocol(e.to_string()))
    }
}
