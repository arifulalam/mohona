//! An SFTP server for tests. Every path ending in `.bin` is a file of `size`
//! bytes of generated content; anything else does not exist.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::net::TcpListener;

use crate::fill_content;

/// How the SFTP server behaves.
#[derive(Clone)]
pub struct SftpConfig {
    /// Size of every `.bin` file.
    pub size: u64,
    /// The only user name accepted.
    pub user: String,
    /// The password accepted (`None`: passwords are refused).
    pub password: Option<String>,
    /// A public key (OpenSSH text) accepted for the user.
    pub authorized_key: Option<String>,
    /// Wait this long after every read answer (to make transfers slow).
    pub read_delay: Option<std::time::Duration>,
}

impl Default for SftpConfig {
    fn default() -> Self {
        Self {
            size: 1_000_000,
            user: "pat".into(),
            password: Some("secret".into()),
            authorized_key: None,
            read_delay: None,
        }
    }
}

/// What a running SFTP server exposes to tests.
#[derive(Clone)]
pub struct SftpHandle {
    /// Where it listens.
    pub addr: SocketAddr,
    /// The server's host key as `SHA256:...`.
    pub fingerprint: String,
    /// The server's host key as OpenSSH public key text.
    pub host_key: String,
    /// TCP connections accepted.
    pub connections: Arc<AtomicUsize>,
    /// Successful logins as `(user, method)`.
    pub logins: Arc<Mutex<Vec<(String, &'static str)>>>,
    /// Every `READ` as `(path, offset)`.
    pub reads: Arc<Mutex<Vec<(String, u64)>>>,
}

/// A new key pair as `(private OpenSSH text, public OpenSSH text)`, with the
/// private key protected by `passphrase` when given.
pub fn generate_key_pair(passphrase: Option<&str>) -> (String, String) {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let public = key.public_key().to_openssh().unwrap();
    let key = match passphrase {
        Some(passphrase) => key.encrypt(&mut rand::rng(), passphrase).unwrap(),
        None => key,
    };
    let private = key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .unwrap();
    (private.to_string(), public)
}

pub async fn start_sftp(config: SftpConfig) -> SftpHandle {
    let host = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let handle_info = (
        host.public_key().fingerprint(HashAlg::Sha256).to_string(),
        host.public_key().to_openssh().unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let handle = SftpHandle {
        addr: listener.local_addr().unwrap(),
        fingerprint: handle_info.0,
        host_key: handle_info.1,
        connections: Arc::default(),
        logins: Arc::default(),
        reads: Arc::default(),
    };
    let ssh_config = Arc::new(russh::server::Config {
        auth_rejection_time: std::time::Duration::from_millis(10),
        auth_rejection_time_initial: Some(std::time::Duration::from_millis(0)),
        keys: vec![host],
        ..Default::default()
    });
    let shared = handle.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            shared.connections.fetch_add(1, Ordering::SeqCst);
            let session = SshSession {
                config: config.clone(),
                shared: shared.clone(),
                channels: HashMap::new(),
                user: String::new(),
            };
            let ssh_config = ssh_config.clone();
            tokio::spawn(async move {
                if let Ok(running) = russh::server::run_stream(ssh_config, stream, session).await {
                    let _ = running.await;
                }
            });
        }
    });
    handle
}

struct SshSession {
    config: SftpConfig,
    shared: SftpHandle,
    channels: HashMap<ChannelId, Channel<Msg>>,
    user: String,
}

impl russh::server::Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        let ok = user == self.config.user && self.config.password.as_deref() == Some(password);
        if ok {
            self.user = user.to_string();
            self.shared
                .logins
                .lock()
                .unwrap()
                .push((user.to_string(), "password"));
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        let allowed = self
            .config
            .authorized_key
            .as_deref()
            .and_then(|text| PublicKey::from_openssh(text).ok());
        if user == self.config.user
            && allowed.is_some_and(|key| key.key_data() == public_key.key_data())
        {
            self.user = user.to_string();
            self.shared
                .logins
                .lock()
                .unwrap()
                .push((user.to_string(), "key"));
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" {
            if let Some(channel) = self.channels.remove(&channel_id) {
                session.channel_success(channel_id)?;
                let files = Files {
                    config: self.config.clone(),
                    shared: self.shared.clone(),
                };
                russh_sftp::server::run(channel.into_stream(), files).await;
            }
        } else {
            session.channel_failure(channel_id)?;
        }
        Ok(())
    }
}

struct Files {
    config: SftpConfig,
    shared: SftpHandle,
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

impl Files {
    fn attributes(&self, path: &str) -> Result<FileAttributes, StatusCode> {
        if path.ends_with(".bin") {
            let mut attributes = FileAttributes {
                size: Some(self.config.size),
                mtime: Some(1_735_689_600),
                atime: Some(1_735_689_600),
                permissions: Some(0o644),
                ..FileAttributes::default()
            };
            attributes.set_regular(true);
            Ok(attributes)
        } else {
            Err(StatusCode::NoSuchFile)
        }
    }
}

impl russh_sftp::server::Handler for Files {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if pflags.contains(OpenFlags::WRITE) {
            return Err(StatusCode::PermissionDenied);
        }
        self.attributes(&filename)?;
        Ok(Handle {
            id,
            handle: filename,
        })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        self.shared.reads.lock().unwrap().push((handle, offset));
        if offset >= self.config.size {
            return Err(StatusCode::Eof);
        }
        let n = usize::try_from((self.config.size - offset).min(u64::from(len))).unwrap();
        let mut data = vec![0u8; n];
        fill_content(offset, &mut data);
        if let Some(delay) = self.config.read_delay {
            tokio::time::sleep(delay).await;
        }
        Ok(Data { id, data })
    }

    async fn close(&mut self, id: u32, _handle: String) -> Result<Status, Self::Error> {
        Ok(ok(id))
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Ok(Attrs {
            id,
            attrs: self.attributes(&path)?,
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        self.stat(id, handle).await
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let absolute = if path == "." || path.is_empty() {
            "/home/pat".to_string()
        } else if path.starts_with('/') {
            path
        } else {
            format!("/home/pat/{path}")
        };
        Ok(Name {
            id,
            files: vec![File::dummy(absolute)],
        })
    }
}
