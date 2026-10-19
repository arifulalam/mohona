//! Small proxies for testing: an HTTP proxy (plain requests and `CONNECT`)
//! and a SOCKS5 proxy. They count what goes through them, so a test can
//! prove that traffic really used the proxy.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A running test proxy.
#[derive(Clone)]
pub struct ProxyHandle {
    /// Where it listens.
    pub addr: SocketAddr,
    /// Connections it relayed (after any login succeeded).
    pub connections: Arc<AtomicUsize>,
    /// `host:port` of each target the client asked for, as the client wrote
    /// it (a name for SOCKS5h, an address when the client looked it up).
    pub targets: Arc<Mutex<Vec<String>>>,
}

impl ProxyHandle {
    /// How many connections went through.
    pub fn relayed(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

type Login = Option<(String, String)>;

fn login(auth: Option<(&str, &str)>) -> Login {
    auth.map(|(u, p)| (u.to_string(), p.to_string()))
}

async fn listen() -> (TcpListener, ProxyHandle) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let handle = ProxyHandle {
        addr: listener.local_addr().unwrap(),
        connections: Arc::default(),
        targets: Arc::default(),
    };
    (listener, handle)
}

/// Start an HTTP proxy. With `auth`, clients must send matching Basic
/// `Proxy-Authorization`, or get `407`.
pub async fn start_http_proxy(auth: Option<(&str, &str)>) -> ProxyHandle {
    let (listener, handle) = listen().await;
    let auth = login(auth);
    let state = handle.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (auth, state) = (auth.clone(), state.clone());
            tokio::spawn(async move {
                let _ = http_proxy(stream, auth, state).await;
            });
        }
    });
    handle
}

async fn read_head(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await? == 0 || head.len() > 64 * 1024 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        head.push(byte[0]);
    }
    Ok(head)
}

async fn http_proxy(mut client: TcpStream, auth: Login, state: ProxyHandle) -> io::Result<()> {
    let head = String::from_utf8_lossy(&read_head(&mut client).await?).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Ok(());
    };
    let headers: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
    if let Some((user, password)) = &auth {
        let expected = format!("Basic {}", STANDARD.encode(format!("{user}:{password}")));
        let given = headers.iter().find_map(|h| {
            let (name, value) = h.split_once(':')?;
            name.eq_ignore_ascii_case("proxy-authorization")
                .then(|| value.trim())
        });
        if given != Some(expected.as_str()) {
            client
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await?;
            return Ok(());
        }
    }
    if method.eq_ignore_ascii_case("CONNECT") {
        let mut upstream = connect_upstream(target).await?;
        state.targets.lock().unwrap().push(target.to_string());
        state.connections.fetch_add(1, Ordering::SeqCst);
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        copy_bidirectional(&mut client, &mut upstream).await?;
        return Ok(());
    }
    // A plain request: `GET http://host:port/path HTTP/1.1`.
    let Some(rest) = target.strip_prefix("http://") else {
        client
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await?;
        return Ok(());
    };
    let (host, path) = rest
        .split_once('/')
        .map_or((rest, "/".to_string()), |(h, p)| (h, format!("/{p}")));
    let host_port = if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    };
    let mut upstream = connect_upstream(&host_port).await?;
    state.targets.lock().unwrap().push(host_port);
    state.connections.fetch_add(1, Ordering::SeqCst);
    let mut forwarded = format!("{method} {path} {version}\r\n");
    for header in headers {
        let name = header.split(':').next().unwrap_or("").to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "proxy-authorization" | "proxy-connection" | "connection"
        ) {
            continue;
        }
        forwarded.push_str(header);
        forwarded.push_str("\r\n");
    }
    forwarded.push_str("Connection: close\r\n\r\n");
    upstream.write_all(forwarded.as_bytes()).await?;
    copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Start a SOCKS5 proxy. With `auth`, clients must log in with user name
/// and password (RFC 1929).
pub async fn start_socks5_proxy(auth: Option<(&str, &str)>) -> ProxyHandle {
    let (listener, handle) = listen().await;
    let auth = login(auth);
    let state = handle.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (auth, state) = (auth.clone(), state.clone());
            tokio::spawn(async move {
                let _ = socks5(stream, auth, state).await;
            });
        }
    });
    handle
}

async fn socks5(mut client: TcpStream, auth: Login, state: ProxyHandle) -> io::Result<()> {
    let mut greeting = [0u8; 2];
    client.read_exact(&mut greeting).await?;
    let mut methods = vec![0u8; usize::from(greeting[1])];
    client.read_exact(&mut methods).await?;
    let wanted = if auth.is_some() { 0x02 } else { 0x00 };
    if greeting[0] != 5 || !methods.contains(&wanted) {
        client.write_all(&[5, 0xFF]).await?;
        return Ok(());
    }
    client.write_all(&[5, wanted]).await?;
    if let Some((user, password)) = &auth {
        let mut version = [0u8; 2];
        client.read_exact(&mut version).await?;
        let mut given_user = vec![0u8; usize::from(version[1])];
        client.read_exact(&mut given_user).await?;
        let mut plen = [0u8; 1];
        client.read_exact(&mut plen).await?;
        let mut given_password = vec![0u8; usize::from(plen[0])];
        client.read_exact(&mut given_password).await?;
        if given_user != user.as_bytes() || given_password != password.as_bytes() {
            client.write_all(&[1, 1]).await?;
            return Ok(());
        }
        client.write_all(&[1, 0]).await?;
    }
    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    let host = match request[3] {
        1 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut name = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        4 => {
            let mut ip = [0u8; 16];
            client.read_exact(&mut ip).await?;
            format!("[{}]", std::net::Ipv6Addr::from(ip))
        }
        _ => {
            client.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await?;
    let target = format!("{host}:{}", u16::from_be_bytes(port));
    if request[1] != 1 {
        client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    }
    let Ok(mut upstream) = connect_upstream(&target).await else {
        client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Ok(());
    };
    state.targets.lock().unwrap().push(target);
    state.connections.fetch_add(1, Ordering::SeqCst);
    client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Connect to a target. The test server listens on IPv4 only, but on some
/// systems (Windows) "localhost" turns into the IPv6 address first, so a
/// refused local connection is tried again on 127.0.0.1.
async fn connect_upstream(target: &str) -> std::io::Result<TcpStream> {
    match TcpStream::connect(target).await {
        Ok(stream) => Ok(stream),
        Err(error) => {
            let port = target.rsplit_once(':').map(|(_, port)| port);
            let local = target.starts_with("localhost:") || target.starts_with("[::1]:");
            match (local, port) {
                (true, Some(port)) => TcpStream::connect(format!("127.0.0.1:{port}")).await,
                _ => Err(error),
            }
        }
    }
}
