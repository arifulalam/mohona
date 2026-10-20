//! A small FTP server (passive mode only) that serves generated files.
//!
//! Every path ending in `.bin` is a file of `size` bytes of generated
//! content; anything else is "not found". It remembers what clients asked.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use crate::tls::Certificates;

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type BoxIo = Box<dyn Io>;

/// Encryption offered by the server.
#[derive(Clone)]
pub struct FtpTls {
    /// The certificates it presents.
    pub certificates: Certificates,
    /// Encrypted from the first byte (`ftps://`), instead of after `AUTH TLS`.
    pub implicit: bool,
}

use crate::fill_content;

/// How the FTP server behaves.
#[derive(Clone)]
pub struct FtpConfig {
    /// Size of every `.bin` file.
    pub size: u64,
    /// Accept only this user and password. `None` accepts anything.
    pub login: Option<(String, String)>,
    /// Refuse `REST` (so downloads cannot be resumed or split).
    pub no_rest: bool,
    /// Answer `421` to connections beyond this many at the same time.
    pub max_connections: Option<usize>,
    /// Do not know `EPSV` (only `PASV`).
    pub no_epsv: bool,
    /// Wait this long after each 32 KiB sent (to make transfers slow enough to
    /// interrupt).
    pub chunk_delay: Option<std::time::Duration>,
    /// Offer encryption. Then `PROT P` encrypts data connections too.
    pub tls: Option<FtpTls>,
}

impl Default for FtpConfig {
    fn default() -> Self {
        Self {
            size: 1_000_000,
            login: None,
            no_rest: false,
            max_connections: None,
            no_epsv: false,
            chunk_delay: None,
            tls: None,
        }
    }
}

/// What a running FTP server exposes to tests.
#[derive(Clone)]
pub struct FtpHandle {
    /// Where it listens.
    pub addr: SocketAddr,
    /// Control connections accepted (including refused ones).
    pub connections: Arc<AtomicUsize>,
    /// Every `RETR` as `(path, rest offset)`.
    pub retrievals: Arc<Mutex<Vec<(String, u64)>>>,
    /// The user names that logged in successfully.
    pub logins: Arc<Mutex<Vec<String>>>,
    /// Data connections that were encrypted.
    pub encrypted_transfers: Arc<AtomicUsize>,
    /// Data connections that were not.
    pub plain_transfers: Arc<AtomicUsize>,
}

pub async fn start_ftp(config: FtpConfig) -> FtpHandle {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let handle = FtpHandle {
        addr: listener.local_addr().unwrap(),
        connections: Arc::default(),
        retrievals: Arc::default(),
        logins: Arc::default(),
        encrypted_transfers: Arc::default(),
        plain_transfers: Arc::default(),
    };
    let active = Arc::new(AtomicUsize::new(0));
    let shared = handle.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            shared.connections.fetch_add(1, Ordering::SeqCst);
            let (config, shared, active) = (config.clone(), shared.clone(), active.clone());
            tokio::spawn(async move {
                let _ = session(stream, config, shared, active).await;
            });
        }
    });
    handle
}

struct Guard(Arc<AtomicUsize>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn session(
    stream: TcpStream,
    config: FtpConfig,
    shared: FtpHandle,
    active: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    let before = active.fetch_add(1, Ordering::SeqCst);
    let _guard = Guard(active);
    let local = stream.local_addr()?.ip();
    let acceptor: Option<TlsAcceptor> = config.tls.as_ref().map(|t| t.certificates.acceptor());
    let implicit = config.tls.as_ref().is_some_and(|t| t.implicit);
    let mut io: BoxIo = Box::new(stream);
    if implicit {
        let Some(acceptor) = &acceptor else {
            return Ok(());
        };
        io = Box::new(acceptor.accept(io).await?);
    }
    let mut lines = BufReader::new(io);
    if config.max_connections.is_some_and(|max| before >= max) {
        lines
            .get_mut()
            .write_all(b"421 Too many connections.\r\n")
            .await?;
        return Ok(());
    }
    lines
        .get_mut()
        .write_all(b"220-Mohona test FTP\r\n220 ready\r\n")
        .await?;
    let mut protect_data = false;

    let (mut user, mut logged_in) = (String::new(), false);
    let mut rest = 0u64;
    let mut pending: Option<TcpListener> = None;
    loop {
        let mut line = String::new();
        if lines.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let line = line.trim_end().to_string();
        let (verb, argument) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let verb = verb.to_ascii_uppercase();
        let reply: String = match verb.as_str() {
            "USER" => {
                user = argument.to_string();
                "331 password please\r\n".into()
            }
            "PASS" => {
                let ok = config
                    .login
                    .as_ref()
                    .is_none_or(|(u, p)| *u == user && p == argument);
                if ok {
                    logged_in = true;
                    shared.logins.lock().unwrap().push(user.clone());
                    "230 welcome\r\n".into()
                } else {
                    "530 Login incorrect.\r\n".into()
                }
            }
            "QUIT" => {
                lines.get_mut().write_all(b"221 bye\r\n").await?;
                return Ok(());
            }
            "AUTH" if acceptor.is_some() && !implicit && argument.eq_ignore_ascii_case("TLS") => {
                lines.get_mut().write_all(b"234 go ahead\r\n").await?;
                let plain =
                    std::mem::replace(lines.get_mut(), Box::new(tokio::io::empty()) as BoxIo);
                let secured = acceptor.as_ref().expect("checked").accept(plain).await?;
                lines = BufReader::new(Box::new(secured));
                continue;
            }
            "PBSZ" if acceptor.is_some() => "200 PBSZ=0\r\n".into(),
            "PROT" if acceptor.is_some() => {
                protect_data = argument.eq_ignore_ascii_case("P");
                "200 ok\r\n".into()
            }
            _ if !logged_in => "530 Please login with USER and PASS.\r\n".into(),
            "TYPE" => "200 type set\r\n".into(),
            "NOOP" => "200 ok\r\n".into(),
            "PWD" => "257 \"/\"\r\n".into(),
            "FEAT" => "211-Features:\r\n SIZE\r\n MDTM\r\n REST STREAM\r\n211 End\r\n".into(),
            "SIZE" => {
                if argument.ends_with(".bin") {
                    format!("213 {}\r\n", config.size)
                } else {
                    "550 No such file.\r\n".into()
                }
            }
            "MDTM" => {
                if argument.ends_with(".bin") {
                    "213 20250101000000\r\n".into()
                } else {
                    "550 No such file.\r\n".into()
                }
            }
            "REST" => {
                if config.no_rest {
                    "502 REST not implemented.\r\n".into()
                } else {
                    rest = argument.parse().unwrap_or(0);
                    "350 restarting\r\n".into()
                }
            }
            "EPSV" | "PASV" => {
                if verb == "EPSV" && config.no_epsv {
                    "500 unknown command\r\n".into()
                } else {
                    let listener = TcpListener::bind((local, 0)).await?;
                    let port = listener.local_addr()?.port();
                    pending = Some(listener);
                    if verb == "EPSV" {
                        format!("229 Entering Extended Passive Mode (|||{port}|)\r\n")
                    } else {
                        // A private address on purpose: clients must not trust it.
                        format!(
                            "227 Entering Passive Mode (10,255,255,1,{},{})\r\n",
                            port / 256,
                            port % 256
                        )
                    }
                }
            }
            "RETR" => {
                shared
                    .retrievals
                    .lock()
                    .unwrap()
                    .push((argument.to_string(), rest));
                match (pending.take(), argument.ends_with(".bin")) {
                    (_, false) => "550 No such file.\r\n".into(),
                    (None, _) => "425 Use PASV first.\r\n".into(),
                    (Some(listener), true) => {
                        lines
                            .get_mut()
                            .write_all(b"150 Opening data connection.\r\n")
                            .await?;
                        let (data, _) = listener.accept().await?;
                        let mut data: BoxIo = if protect_data {
                            let Some(acceptor) = &acceptor else {
                                return Ok(());
                            };
                            shared.encrypted_transfers.fetch_add(1, Ordering::SeqCst);
                            Box::new(acceptor.accept(data).await?)
                        } else {
                            shared.plain_transfers.fetch_add(1, Ordering::SeqCst);
                            Box::new(data)
                        };
                        let mut offset = rest.min(config.size);
                        rest = 0;
                        let mut buffer = vec![0u8; 32 * 1024];
                        let mut complete = true;
                        while offset < config.size {
                            let n =
                                usize::try_from((config.size - offset).min(buffer.len() as u64))
                                    .unwrap();
                            fill_content(offset, &mut buffer[..n]);
                            if data.write_all(&buffer[..n]).await.is_err() {
                                complete = false;
                                break;
                            }
                            offset += n as u64;
                            if let Some(delay) = config.chunk_delay {
                                tokio::time::sleep(delay).await;
                            }
                        }
                        let _ = data.shutdown().await;
                        drop(data);
                        if complete {
                            "226 Transfer complete.\r\n".into()
                        } else {
                            "426 Connection closed; transfer aborted.\r\n".into()
                        }
                    }
                }
            }
            _ => "502 not implemented\r\n".into(),
        };
        lines.get_mut().write_all(reply.as_bytes()).await?;
    }
}
