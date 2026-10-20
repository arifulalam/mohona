//! FTP: probing, reading a file, and reading a part of it.
//!
//! Plain FTP in passive mode. Every request opens its own control
//! connection, logs in, and (for data) its own data connection, like an
//! HTTP request opens its own range request. That keeps segments independent.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use percent_encoding::percent_decode_str;
use reqwest::Url;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::http::{ClientConfig, Probe};
use crate::{Credentials, NetError};

/// What an FTP address names.
struct Target {
    host: String,
    port: u16,
    path: String,
    user: String,
    password: String,
}

fn target(url: &Url, credentials: Option<&Credentials>) -> Result<Target, NetError> {
    let host = url
        .host_str()
        .ok_or_else(|| NetError::InvalidUrl(format!("{url} (no host)")))?
        .trim_matches(['[', ']'])
        .to_string();
    let decode = |text: &str| percent_decode_str(text).decode_utf8_lossy().into_owned();
    let (user, password) = if let Some(c) = credentials {
        (c.username.clone(), c.password.clone())
    } else if !url.username().is_empty() {
        (
            decode(url.username()),
            decode(url.password().unwrap_or_default()),
        )
    } else {
        // The usual way into a public server.
        ("anonymous".to_string(), "mohona@".to_string())
    };
    let path = decode(url.path());
    if path.is_empty() || path.ends_with('/') {
        return Err(NetError::InvalidUrl(format!(
            "{url} (the address must name a file, not a folder)"
        )));
    }
    if path.contains(['\r', '\n']) {
        return Err(NetError::InvalidUrl(format!("{url} (bad characters)")));
    }
    Ok(Target {
        host,
        port: url.port().unwrap_or(21),
        path,
        user,
        password,
    })
}

fn io_error(error: std::io::Error) -> NetError {
    NetError::FtpIo(error)
}

/// One FTP control connection, logged in.
struct Control {
    reader: BufReader<TcpStream>,
    peer: IpAddr,
    wait: Duration,
}

impl Control {
    async fn connect(target: &Target, config: &ClientConfig) -> Result<Self, NetError> {
        let stream = timeout(
            config.connect_timeout,
            TcpStream::connect((target.host.as_str(), target.port)),
        )
        .await
        .map_err(|_| io_error(std::io::ErrorKind::TimedOut.into()))?
        .map_err(io_error)?;
        let _ = stream.set_nodelay(true);
        let peer = stream.peer_addr().map_err(io_error)?.ip();
        let mut control = Self {
            reader: BufReader::new(stream),
            peer,
            wait: config.read_timeout,
        };
        let (code, text) = control.reply().await?;
        if code != 220 {
            return Err(NetError::Ftp {
                code,
                message: text,
            });
        }
        let (code, text) = control.command(&format!("USER {}", target.user)).await?;
        match code {
            230 => {}
            331 | 332 => {
                let (code, text) = control
                    .command(&format!("PASS {}", target.password))
                    .await?;
                if code != 230 {
                    return Err(NetError::Ftp {
                        code,
                        message: text,
                    });
                }
            }
            _ => {
                return Err(NetError::Ftp {
                    code,
                    message: text,
                })
            }
        }
        Ok(control)
    }

    /// Read one reply, which may span several lines (`123-` ... `123 `).
    async fn reply(&mut self) -> Result<(u16, String), NetError> {
        let mut first: Option<(u16, String)> = None;
        loop {
            let mut line = String::new();
            let read = timeout(self.wait, self.reader.read_line(&mut line))
                .await
                .map_err(|_| io_error(std::io::ErrorKind::TimedOut.into()))?
                .map_err(io_error)?;
            if read == 0 {
                return Err(io_error(std::io::ErrorKind::UnexpectedEof.into()));
            }
            let line = line.trim_end();
            let code = line.get(..3).and_then(|c| c.parse::<u16>().ok());
            let (Some(code), Some(rest)) = (code, line.get(3..)) else {
                continue; // a continuation line without a code
            };
            let (last, text) = match rest.chars().next() {
                Some('-') => (false, rest[1..].trim()),
                _ => (true, rest.trim()),
            };
            match &first {
                None => first = Some((code, text.to_string())),
                Some((first_code, _)) if *first_code != code => continue,
                _ => {}
            }
            if last {
                return Ok(first.expect("set above"));
            }
        }
    }

    async fn command(&mut self, command: &str) -> Result<(u16, String), NetError> {
        let stream = self.reader.get_mut();
        stream
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .map_err(io_error)?;
        self.reply().await
    }

    /// Send a command that must answer with one of `ok`.
    async fn expect(&mut self, command: &str, ok: &[u16]) -> Result<String, NetError> {
        let (code, text) = self.command(command).await?;
        if ok.contains(&code) {
            Ok(text)
        } else {
            Err(NetError::Ftp {
                code,
                message: text,
            })
        }
    }

    /// Open a passive data connection.
    async fn passive(&mut self, config: &ClientConfig) -> Result<TcpStream, NetError> {
        let port = match self.command("EPSV").await? {
            (229, text) => parse_epsv(&text),
            _ => None,
        };
        let address = match port {
            Some(port) => SocketAddr::new(self.peer, port),
            None => {
                // Older servers: PASV. Its address is ignored (it is often a
                // private one behind a NAT); the control connection's is used.
                let text = self.expect("PASV", &[227]).await?;
                let port = parse_pasv(&text).ok_or(NetError::Ftp {
                    code: 227,
                    message: format!("cannot read the data address: {text}"),
                })?;
                SocketAddr::new(self.peer, port)
            }
        };
        let stream = timeout(config.connect_timeout, TcpStream::connect(address))
            .await
            .map_err(|_| io_error(std::io::ErrorKind::TimedOut.into()))?
            .map_err(io_error)?;
        Ok(stream)
    }

    async fn quit(mut self) {
        let _ = self.command("QUIT").await;
    }
}

/// `Entering Extended Passive Mode (|||6446|)`.
fn parse_epsv(text: &str) -> Option<u16> {
    let inner = text.split_once('(')?.1.split_once(')')?.0;
    inner.split('|').rev().nth(1)?.parse().ok()
}

/// `Entering Passive Mode (192,168,0,1,19,142)`.
fn parse_pasv(text: &str) -> Option<u16> {
    let inner = text.split_once('(')?.1.split_once(')')?.0;
    let numbers: Vec<u16> = inner
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    (numbers.len() == 6).then(|| numbers[4] * 256 + numbers[5])
}

/// `20250101120000` or `20250101120000.123` to a readable validator.
fn mdtm_validator(text: &str) -> String {
    format!("ftp-mdtm:{}", text.trim())
}

/// What the server says about the file.
pub(crate) async fn probe(
    url: &Url,
    credentials: Option<&Credentials>,
    config: &ClientConfig,
) -> Result<Probe, NetError> {
    let target = target(url, credentials)?;
    let mut control = Control::connect(&target, config).await?;
    control.expect("TYPE I", &[200]).await?;
    let size = match control.command(&format!("SIZE {}", target.path)).await? {
        (213, text) => text.trim().parse::<u64>().ok(),
        (550, message) => return Err(NetError::Ftp { code: 550, message }),
        _ => None,
    };
    let last_modified = match control.command(&format!("MDTM {}", target.path)).await? {
        (213, text) => Some(mdtm_validator(&text)),
        _ => None,
    };
    // Resuming works if the server accepts REST.
    let accepts_ranges = matches!(control.command("REST 0").await?, (350, _));
    control.quit().await;
    let filename = url
        .path_segments()
        .and_then(|mut s| s.next_back().map(str::to_string))
        .map(|s| percent_decode_str(&s).decode_utf8_lossy().into_owned())
        .filter(|s| !s.is_empty());
    Ok(Probe {
        final_url: url.to_string(),
        size,
        accepts_ranges: accepts_ranges && size.is_some(),
        etag: None,
        last_modified,
        content_type: None,
        filename,
        digests: Vec::new(),
    })
}

/// A file being read from an FTP server.
pub(crate) struct FtpBody {
    data: TcpStream,
    control: Option<Control>,
    /// Bytes still wanted, when only a part was asked for.
    remaining: Option<u64>,
    wait: Duration,
    done: bool,
}

impl FtpBody {
    pub(crate) fn content_length(&self) -> Option<u64> {
        self.remaining
    }

    pub(crate) async fn chunk(&mut self) -> Result<Option<Bytes>, NetError> {
        if self.done {
            return Ok(None);
        }
        if self.remaining == Some(0) {
            self.finish(false).await?;
            return Ok(None);
        }
        let mut buffer = BytesMut::with_capacity(64 * 1024);
        let read = timeout(self.wait, self.data.read_buf(&mut buffer))
            .await
            .map_err(|_| io_error(std::io::ErrorKind::TimedOut.into()))?
            .map_err(io_error)?;
        if read == 0 {
            // The transfer ended: the server confirms or complains.
            self.finish(true).await?;
            return Ok(None);
        }
        if let Some(remaining) = self.remaining.as_mut() {
            if read as u64 > *remaining {
                buffer.truncate(usize::try_from(*remaining).unwrap_or(read));
            }
            *remaining -= buffer.len() as u64;
        }
        Ok(Some(buffer.freeze()))
    }

    /// End the transfer. When the whole file was read, wait for the server's
    /// verdict; when only a part was wanted, just hang up.
    async fn finish(&mut self, wait_for_reply: bool) -> Result<(), NetError> {
        self.done = true;
        let Some(mut control) = self.control.take() else {
            return Ok(());
        };
        if wait_for_reply {
            let (code, message) = control.reply().await?;
            if code != 226 && code != 250 {
                return Err(NetError::Ftp { code, message });
            }
            control.quit().await;
        }
        Ok(())
    }
}

/// Start reading from byte `first` on (to the end, or for `length` bytes).
pub(crate) async fn open(
    url: &Url,
    credentials: Option<&Credentials>,
    config: &ClientConfig,
    first: u64,
    length: Option<u64>,
) -> Result<FtpBody, NetError> {
    let target = target(url, credentials)?;
    let mut control = Control::connect(&target, config).await?;
    control.expect("TYPE I", &[200]).await?;
    let data = control.passive(config).await?;
    if first > 0 {
        let (code, message) = control.command(&format!("REST {first}")).await?;
        if code != 350 {
            return Err(NetError::WrongRange(format!(
                "the FTP server cannot continue from byte {first} ({code} {message})"
            )));
        }
    }
    let (code, message) = control.command(&format!("RETR {}", target.path)).await?;
    if code != 125 && code != 150 {
        return Err(NetError::Ftp { code, message });
    }
    Ok(FtpBody {
        data,
        wait: config.read_timeout,
        control: Some(control),
        remaining: length,
        done: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_passive_replies() {
        assert_eq!(
            parse_epsv("Entering Extended Passive Mode (|||6446|)"),
            Some(6446)
        );
        assert_eq!(parse_epsv("nonsense"), None);
        assert_eq!(
            parse_pasv("Entering Passive Mode (192,168,0,1,19,142)"),
            Some(19 * 256 + 142)
        );
        assert_eq!(parse_pasv("Entering Passive Mode (1,2,3)"), None);
    }

    #[test]
    fn reads_addresses() {
        let t = |s: &str, c: Option<&Credentials>| target(&Url::parse(s).unwrap(), c);
        let anonymous = t("ftp://host/pub/a%20b.iso", None).unwrap();
        assert_eq!((anonymous.user.as_str(), anonymous.port), ("anonymous", 21));
        assert_eq!(anonymous.path, "/pub/a b.iso");
        let named = t("ftp://me:p%40ss@host:2121/x.bin", None).unwrap();
        assert_eq!(
            (named.user.as_str(), named.password.as_str(), named.port),
            ("me", "p@ss", 2121)
        );
        assert!(t("ftp://host/dir/", None).is_err());
        assert!(t("ftp://host", None).is_err());
        assert!(t("ftp://host/a%0D%0ADELE%20b", None).is_err());
    }
}
