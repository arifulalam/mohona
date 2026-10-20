//! SFTP (file transfer over SSH): probing, reading a file, and reading a part.
//!
//! Like FTP, every request opens its own connection and logs in, so segments
//! are independent. The server's host key is checked before any password or
//! key is offered.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use percent_encoding::percent_decode_str;
use reqwest::Url;
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey};
use russh::Disconnect;
use russh_sftp::client::fs::File;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::StatusCode;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::http::{ClientConfig, Probe};
use crate::{Credentials, NetError, SshOptions};

/// The environment variable that holds a key's passphrase. It is never saved.
pub const PASSPHRASE_VARIABLE: &str = "MOHONA_KEY_PASSPHRASE";

struct Target {
    host: String,
    port: u16,
    path: String,
    user: String,
    password: Option<String>,
}

fn target(url: &Url, credentials: Option<&Credentials>) -> Result<Target, NetError> {
    let decode = |text: &str| percent_decode_str(text).decode_utf8_lossy().into_owned();
    let host = url
        .host_str()
        .ok_or_else(|| NetError::InvalidUrl(format!("{url} (no host)")))?
        .trim_matches(['[', ']'])
        .to_string();
    let (user, password) = match credentials {
        Some(c) => (c.username.clone(), Some(c.password.clone())),
        None if !url.username().is_empty() => (decode(url.username()), url.password().map(decode)),
        None => {
            return Err(NetError::SshLogin(
                "SFTP needs a user name: sftp://user@host/path, or -u user:password".into(),
            ))
        }
    };
    let path = decode(url.path());
    if path.is_empty() || path.ends_with('/') {
        return Err(NetError::InvalidUrl(format!(
            "{url} (the address must name a file, not a folder)"
        )));
    }
    Ok(Target {
        host,
        port: url.port().unwrap_or(22),
        path,
        user,
        password: password.filter(|p| !p.is_empty()),
    })
}

/// Why a server's key was not accepted.
#[derive(Debug)]
enum KeyProblem {
    Unknown {
        fingerprint: String,
    },
    Changed {
        fingerprint: String,
    },
    Pinned {
        expected: String,
        fingerprint: String,
    },
    File(String),
}

fn key_error(problem: KeyProblem, host: &str) -> NetError {
    NetError::HostKey(match problem {
        KeyProblem::Unknown { fingerprint } => format!(
            "{host} is not a known server (its key is {fingerprint}). If that is the key you expect, \
             trust it with --host-key {fingerprint}, or with --trust-new-host to remember it"
        ),
        KeyProblem::Changed { fingerprint } => format!(
            "the key of {host} is not the one recorded earlier (it is now {fingerprint}). \
             The server may have been reinstalled, or someone may be pretending to be it; \
             check with its owner before changing your known_hosts file"
        ),
        KeyProblem::Pinned {
            expected,
            fingerprint,
        } => format!(
            "the key of {host} is {fingerprint}, not the {expected} you gave with --host-key"
        ),
        KeyProblem::File(reason) => format!("cannot check {host} against known_hosts: {reason}"),
    })
}

/// Decides whether to trust the server's key.
struct HostCheck {
    host: String,
    port: u16,
    options: SshOptions,
    accept_any: bool,
    problem: Arc<Mutex<Option<KeyProblem>>>,
}

fn known_hosts_file(options: &SshOptions) -> Option<PathBuf> {
    match &options.known_hosts {
        Some(path) => Some(PathBuf::from(path)),
        None => std::env::home_dir().map(|home| home.join(".ssh").join("known_hosts")),
    }
}

impl HostCheck {
    fn judge(&self, key: &PublicKey) -> Result<(), KeyProblem> {
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        if self.accept_any {
            return Ok(());
        }
        if let Some(expected) = &self.options.host_key {
            let normal = |s: &str| s.trim().trim_start_matches("SHA256:").to_string();
            return if normal(expected) == normal(&fingerprint) {
                Ok(())
            } else {
                Err(KeyProblem::Pinned {
                    expected: expected.clone(),
                    fingerprint,
                })
            };
        }
        let Some(path) = known_hosts_file(&self.options) else {
            return Err(KeyProblem::File("there is no home folder".into()));
        };
        let known = if path.exists() {
            match russh::keys::known_hosts::check_known_hosts_path(
                &self.host, self.port, key, &path,
            ) {
                Ok(known) => known,
                Err(russh::keys::Error::KeyChanged { .. }) => {
                    return Err(KeyProblem::Changed { fingerprint })
                }
                Err(error) => return Err(KeyProblem::File(error.to_string())),
            }
        } else {
            false
        };
        if known {
            Ok(())
        } else if self.options.trust_new_host {
            russh::keys::known_hosts::learn_known_hosts_path(&self.host, self.port, key, &path)
                .map_err(|e| KeyProblem::File(e.to_string()))
        } else {
            Err(KeyProblem::Unknown { fingerprint })
        }
    }
}

impl client::Handler for HostCheck {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            return Ok(false);
        };
        match self.judge(key) {
            Ok(()) => Ok(true),
            Err(problem) => {
                *self.problem.lock().unwrap_or_else(|e| e.into_inner()) = Some(problem);
                Ok(false)
            }
        }
    }
}

fn ssh_error(error: impl std::fmt::Display) -> NetError {
    NetError::Ssh(error.to_string())
}

/// A logged-in SFTP session and the connection it lives on.
struct Connected {
    sftp: SftpSession,
    handle: Handle<HostCheck>,
}

impl Connected {
    async fn end(self) {
        let _ = self.sftp.close().await;
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
    }
}

fn load_key(path: &str) -> Result<PrivateKey, NetError> {
    let passphrase = std::env::var(PASSPHRASE_VARIABLE).ok();
    russh::keys::load_secret_key(path, passphrase.as_deref()).map_err(|error| {
        let hint = if passphrase.is_none() {
            format!(" (if the key has a passphrase, set {PASSPHRASE_VARIABLE})")
        } else {
            String::new()
        };
        NetError::SshLogin(format!("cannot use the key file {path}: {error}{hint}"))
    })
}

fn default_keys() -> Vec<PrivateKey> {
    let Some(home) = std::env::home_dir() else {
        return Vec::new();
    };
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|name| home.join(".ssh").join(name))
        .filter(|path| path.exists())
        .filter_map(|path| russh::keys::load_secret_key(path, None).ok())
        .collect()
}

async fn connect(
    target: &Target,
    config: &ClientConfig,
    options: &SshOptions,
    accept_any_host: bool,
) -> Result<Connected, NetError> {
    let problem = Arc::new(Mutex::new(None));
    let handler = HostCheck {
        host: target.host.clone(),
        port: target.port,
        options: options.clone(),
        accept_any: accept_any_host,
        problem: problem.clone(),
    };
    let stream = timeout(
        config.connect_timeout,
        TcpStream::connect((target.host.as_str(), target.port)),
    )
    .await
    .map_err(|_| NetError::Connection(std::io::ErrorKind::TimedOut.into()))?
    .map_err(NetError::Connection)?;
    let _ = stream.set_nodelay(true);
    let ssh_config = Arc::new(client::Config {
        inactivity_timeout: Some(config.read_timeout.max(Duration::from_secs(10))),
        ..Default::default()
    });
    let mut handle = match timeout(
        config.read_timeout,
        client::connect_stream(ssh_config, stream, handler),
    )
    .await
    {
        Err(_) => return Err(NetError::Connection(std::io::ErrorKind::TimedOut.into())),
        Ok(Ok(handle)) => handle,
        Ok(Err(error)) => {
            let found = problem.lock().unwrap_or_else(|e| e.into_inner()).take();
            return Err(match found {
                Some(found) => key_error(found, &target.host),
                None => ssh_error(error),
            });
        }
    };

    // Offer a key first when one was named, then the password.
    let mut authenticated = false;
    let mut tried_anything = false;
    let mut keys: Vec<PrivateKey> = Vec::new();
    if let Some(path) = &options.key_file {
        keys.push(load_key(path)?);
    } else if target.password.is_none() {
        keys = default_keys();
    }
    for key in keys {
        tried_anything = true;
        let hash = handle
            .best_supported_rsa_hash()
            .await
            .map_err(ssh_error)?
            .flatten();
        let result = handle
            .authenticate_publickey(
                target.user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await
            .map_err(ssh_error)?;
        if result.success() {
            authenticated = true;
            break;
        }
    }
    if !authenticated {
        if let Some(password) = &target.password {
            tried_anything = true;
            let result = handle
                .authenticate_password(target.user.clone(), password.clone())
                .await
                .map_err(ssh_error)?;
            authenticated = result.success();
        }
    }
    if !authenticated {
        return Err(NetError::SshLogin(if tried_anything {
            format!(
                "the server refused the login for {} (wrong password or key?)",
                target.user
            )
        } else {
            "no login to offer: give -u user:password, or --key FILE".into()
        }));
    }

    let channel = handle.channel_open_session().await.map_err(ssh_error)?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| ssh_error(format!("the server does not offer SFTP ({e})")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| ssh_error(format!("SFTP did not start ({e})")))?;
    Ok(Connected { sftp, handle })
}

/// Turn an SFTP error into ours.
fn sftp_error(error: russh_sftp::client::error::Error, path: &str) -> NetError {
    use russh_sftp::client::error::Error;
    match error {
        Error::Status(status) => {
            let fatal = matches!(
                status.status_code,
                StatusCode::NoSuchFile
                    | StatusCode::PermissionDenied
                    | StatusCode::OpUnsupported
                    | StatusCode::BadMessage
            );
            let what = match status.status_code {
                StatusCode::NoSuchFile => "the file is not there".to_string(),
                StatusCode::PermissionDenied => "you may not read it".to_string(),
                _ => status.error_message.clone(),
            };
            NetError::Sftp {
                message: format!("{path}: {what}"),
                fatal,
            }
        }
        other => NetError::Sftp {
            message: format!("{path}: {other}"),
            fatal: false,
        },
    }
}

/// What the server says about the file.
pub(crate) async fn probe(
    url: &Url,
    credentials: Option<&Credentials>,
    config: &ClientConfig,
    options: &SshOptions,
    accept_any_host: bool,
) -> Result<Probe, NetError> {
    let target = target(url, credentials)?;
    let connected = connect(&target, config, options, accept_any_host).await?;
    let meta = connected.sftp.metadata(target.path.clone()).await;
    connected.end().await;
    let meta = meta.map_err(|e| sftp_error(e, &target.path))?;
    if meta.is_dir() {
        return Err(NetError::InvalidUrl(format!(
            "{url} (the address must name a file, not a folder)"
        )));
    }
    let filename = url
        .path_segments()
        .and_then(|mut s| s.next_back().map(str::to_string))
        .map(|s| percent_decode_str(&s).decode_utf8_lossy().into_owned())
        .filter(|s| !s.is_empty());
    Ok(Probe {
        final_url: url.to_string(),
        size: meta.size,
        accepts_ranges: meta.size.is_some(),
        etag: None,
        last_modified: meta
            .mtime
            .map(|mtime| format!("sftp-mtime:{mtime}-{}", meta.size.unwrap_or(0))),
        content_type: None,
        filename,
        digests: Vec::new(),
    })
}

/// A file being read over SFTP.
pub(crate) struct SftpBody {
    file: Option<File>,
    connected: Option<Connected>,
    remaining: Option<u64>,
    wait: Duration,
    path: String,
}

impl SftpBody {
    pub(crate) fn content_length(&self) -> Option<u64> {
        self.remaining
    }

    pub(crate) async fn chunk(&mut self) -> Result<Option<Bytes>, NetError> {
        if self.remaining == Some(0) {
            self.finish().await;
            return Ok(None);
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(None);
        };
        let mut buffer = BytesMut::with_capacity(64 * 1024);
        let read = timeout(self.wait, file.read_buf(&mut buffer))
            .await
            .map_err(|_| NetError::Connection(std::io::ErrorKind::TimedOut.into()))?
            .map_err(|e| NetError::Sftp {
                message: format!("{}: {e}", self.path),
                fatal: false,
            })?;
        if read == 0 {
            self.finish().await;
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

    async fn finish(&mut self) {
        self.file = None;
        if let Some(connected) = self.connected.take() {
            connected.end().await;
        }
    }
}

/// Start reading from byte `first` on (to the end, or for `length` bytes).
pub(crate) async fn open(
    url: &Url,
    credentials: Option<&Credentials>,
    config: &ClientConfig,
    options: &SshOptions,
    accept_any_host: bool,
    first: u64,
    length: Option<u64>,
) -> Result<SftpBody, NetError> {
    let target = target(url, credentials)?;
    let connected = connect(&target, config, options, accept_any_host).await?;
    let opened = connected.sftp.open(target.path.clone()).await;
    let mut file = match opened {
        Ok(file) => file,
        Err(error) => {
            connected.end().await;
            return Err(sftp_error(error, &target.path));
        }
    };
    if first > 0 {
        if let Err(error) = file.seek(std::io::SeekFrom::Start(first)).await {
            connected.end().await;
            return Err(NetError::WrongRange(format!(
                "cannot continue from byte {first} ({error})"
            )));
        }
    }
    Ok(SftpBody {
        file: Some(file),
        connected: Some(connected),
        remaining: length,
        wait: config.read_timeout,
        path: target.path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_addresses() {
        let t = |s: &str, c: Option<&Credentials>| target(&Url::parse(s).unwrap(), c);
        let named = t("sftp://pat:p%40ss@host:2222/home/pat/a%20b.bin", None).unwrap();
        assert_eq!(
            (named.user.as_str(), named.password.as_deref(), named.port),
            ("pat", Some("p@ss"), 2222)
        );
        assert_eq!(named.path, "/home/pat/a b.bin");
        assert_eq!(t("sftp://pat@host/x.bin", None).unwrap().port, 22);
        assert!(
            t("sftp://host/x.bin", None).is_err(),
            "a user name is needed"
        );
        assert!(t("sftp://pat@host/dir/", None).is_err());
        let creds = Credentials {
            username: "me".into(),
            password: "pw".into(),
        };
        assert_eq!(t("sftp://host/x.bin", Some(&creds)).unwrap().user, "me");
    }
}
