//! What a download sends with every request besides the address: cookies,
//! the page it came from, a browser identity and any other headers.

use std::fmt;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, COOKIE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};

use crate::{Checksum, Credentials, NetError};

/// Which proxy a download goes through, instead of the system's.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyChoice {
    /// No proxy at all, even if the system has one.
    Direct,
    /// This proxy: `http://host:port`, `https://host:port`, `socks5://host:port`
    /// or `socks5h://host:port` (`socks5h` also looks names up through the
    /// proxy). A `user:password@` in the address is the proxy's login.
    Url(String),
}

impl fmt::Debug for ProxyChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyChoice::Direct => f.write_str("Direct"),
            // The address may hold a password.
            ProxyChoice::Url(_) => f.write_str("Url(<hidden>)"),
        }
    }
}

impl ProxyChoice {
    /// Read what a person typed: `direct`/`none`/`off`, or a proxy address.
    /// An address without a scheme is taken as an HTTP proxy.
    pub fn parse(text: &str) -> Result<Self, NetError> {
        let text = text.trim();
        if ["direct", "none", "off"]
            .iter()
            .any(|word| text.eq_ignore_ascii_case(word))
        {
            return Ok(ProxyChoice::Direct);
        }
        let full = if text.contains("://") {
            text.to_string()
        } else {
            format!("http://{text}")
        };
        let url = reqwest::Url::parse(&full)
            .map_err(|e| NetError::InvalidProxy(format!("{text:?} ({e})")))?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
            return Err(NetError::InvalidProxy(format!(
                "{text:?}: use http://, https://, socks5:// or socks5h://"
            )));
        }
        if url.host_str().is_none() {
            return Err(NetError::InvalidProxy(format!("{text:?} has no host")));
        }
        Ok(ProxyChoice::Url(full))
    }
}

/// Extra request details for one download. The browser extension fills this
/// in from the page a download was started on; people can also give it on
/// the command line.
///
/// Cookies and header values are secrets: [`Debug`](fmt::Debug) never prints
/// them.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestContext {
    /// The value of the `Cookie` header, like `session=abc; theme=dark`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookies: Option<String>,
    /// The page the download link was on (`Referer`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    /// Replaces the client's default `User-Agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Any other headers, as (name, value) pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    /// A user name and password, sent only when a server asks for them
    /// (Basic or Digest), and only to that server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Credentials>,
    /// A proxy for this download, or `Direct` for none. Without it the
    /// system's proxy settings apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyChoice>,
    /// How to check the server's certificate. Without it, the computer's
    /// trusted authorities are used and anything else is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsOptions>,
    /// How to trust the server and log in for SFTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshOptions>,
    /// What the finished file must hash to. A file that does not match
    /// fails the download (and is kept).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<Checksum>,
    /// Look for a checksum file next to the download (`<address>.sha256`,
    /// `.sha512`, `.sha1`, `.md5`) and check against it if there is one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub find_checksum: bool,
    /// Other addresses of the same file. The download uses them together
    /// with the main address, and moves on to another when one fails.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<String>,
    /// Use at most this many connections for this download (a site that
    /// bans many connections); without it the engine's setting applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connections: Option<u32>,
    /// A torrent: only these files (numbers from the torrent's file list); without it, all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_files: Option<Vec<u32>>,
}

/// SSH settings for one download. A key's passphrase is not part of this: it
/// is read from the environment when needed and never saved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshOptions {
    /// A private key file to log in with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_file: Option<String>,
    /// Trust only the server whose key has this fingerprint (`SHA256:...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key: Option<String>,
    /// The file of known servers (default: `.ssh/known_hosts` in the home folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_hosts: Option<String>,
    /// Remember a server seen for the first time (in the known-hosts file).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trust_new_host: bool,
}

/// Changes to the certificate check, for one download.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsOptions {
    /// Accept any certificate, even a wrong or expired one. Unsafe: anyone on
    /// the way can read and change the download.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub insecure: bool,
    /// A file (PEM) with an extra certificate authority to trust.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<String>,
}

impl fmt::Debug for RequestContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestContext")
            .field("cookies", &self.cookies.as_ref().map(|_| "<hidden>"))
            .field("referrer", &self.referrer.as_ref().map(|_| "<hidden>"))
            .field("user_agent", &self.user_agent)
            .field("credentials", &self.credentials)
            .field("proxy", &self.proxy)
            .field("tls", &self.tls)
            .field("ssh", &self.ssh)
            .field("checksum", &self.checksum)
            .field("find_checksum", &self.find_checksum)
            .field("mirrors", &self.mirrors.len())
            .field(
                "headers",
                &self.headers.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Headers the transport must control itself: setting them from outside
/// would break ranges, framing or connection handling.
const FORBIDDEN: &[&str] = &[
    "range",
    "if-range",
    "content-length",
    "transfer-encoding",
    "host",
    "connection",
    "accept-encoding",
    "upgrade",
];

impl RequestContext {
    /// Nothing extra.
    pub fn is_empty(&self) -> bool {
        self.cookies.is_none()
            && self.referrer.is_none()
            && self.user_agent.is_none()
            && self.headers.is_empty()
            && self.credentials.is_none()
            && self.proxy.is_none()
            && self.tls.is_none()
            && self.ssh.is_none()
            && self.checksum.is_none()
            && !self.find_checksum
            && self.mirrors.is_empty()
    }

    /// Parse `Name: value` (the way `curl -H` takes it) and add it.
    pub fn add_header_line(&mut self, line: &str) -> Result<(), NetError> {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| NetError::InvalidHeader(format!("{line:?} is not like Name: value")))?;
        self.headers
            .push((name.trim().to_string(), value.trim().to_string()));
        self.header_map().map(|_| ())
    }

    /// The headers as they will be sent. Fails on a bad name or value, or a
    /// header the transport controls itself.
    pub(crate) fn header_map(&self) -> Result<HeaderMap, NetError> {
        let mut map = HeaderMap::new();
        let bad =
            |what: &str, why: &dyn fmt::Display| NetError::InvalidHeader(format!("{what}: {why}"));
        for (name, value) in &self.headers {
            let header = HeaderName::from_bytes(name.as_bytes()).map_err(|e| bad(name, &e))?;
            if FORBIDDEN.contains(&header.as_str()) {
                return Err(NetError::InvalidHeader(format!(
                    "{name} is set by Mohona itself and cannot be changed"
                )));
            }
            let mut value = HeaderValue::from_str(value).map_err(|e| bad(name, &e))?;
            value.set_sensitive(is_secret(&header));
            map.append(header, value);
        }
        if let Some(cookies) = &self.cookies {
            let mut value = HeaderValue::from_str(cookies).map_err(|e| bad("cookie", &e))?;
            value.set_sensitive(true);
            map.insert(COOKIE, value);
        }
        if let Some(referrer) = &self.referrer {
            map.insert(
                REFERER,
                HeaderValue::from_str(referrer).map_err(|e| bad("referer", &e))?,
            );
        }
        if let Some(agent) = &self.user_agent {
            map.insert(
                USER_AGENT,
                HeaderValue::from_str(agent).map_err(|e| bad("user-agent", &e))?,
            );
        }
        Ok(map)
    }

    /// Save as JSON (for the database).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// Read what [`RequestContext::to_json`] wrote.
    pub fn from_json(text: &str) -> Result<Self, NetError> {
        serde_json::from_str(text).map_err(|e| NetError::InvalidHeader(e.to_string()))
    }
}

fn is_secret(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "x-api-key" | "x-auth-token"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_headers() {
        let mut c = RequestContext {
            cookies: Some("a=1; b=2".into()),
            referrer: Some("https://example.org/page".into()),
            user_agent: Some("Test/1".into()),
            ..RequestContext::default()
        };
        c.add_header_line("X-Token: abc").unwrap();
        c.add_header_line("accept-language:  bn ").unwrap();
        let map = c.header_map().unwrap();
        assert_eq!(map["cookie"], "a=1; b=2");
        assert_eq!(map["referer"], "https://example.org/page");
        assert_eq!(map["user-agent"], "Test/1");
        assert_eq!(map["x-token"], "abc");
        assert_eq!(map["accept-language"], "bn");
        assert!(map["cookie"].is_sensitive());
    }

    #[test]
    fn refuses_bad_and_forbidden_headers() {
        let mut c = RequestContext::default();
        assert!(c.add_header_line("no colon here").is_err());
        assert!(c.add_header_line("Bad Name: x").is_err());
        assert!(c.add_header_line("X: line\nbreak").is_err());
        for forbidden in ["Range: bytes=0-1", "Host: evil", "Accept-Encoding: gzip"] {
            let mut c = RequestContext::default();
            assert!(c.add_header_line(forbidden).is_err(), "{forbidden}");
        }
    }

    #[test]
    fn json_round_trip_and_defaults() {
        let c = RequestContext {
            cookies: Some("s=1".into()),
            headers: vec![("X-A".into(), "b".into())],
            ..RequestContext::default()
        };
        assert_eq!(RequestContext::from_json(&c.to_json()).unwrap(), c);
        assert!(RequestContext::from_json("{}").unwrap().is_empty());
        assert!(RequestContext::default().is_empty());
    }

    #[test]
    fn debug_hides_secrets() {
        let c = RequestContext {
            cookies: Some("secret=zzz".into()),
            headers: vec![("Authorization".into(), "Bearer topsecret".into())],
            ..RequestContext::default()
        };
        let text = format!("{c:?}");
        assert!(
            !text.contains("zzz") && !text.contains("topsecret"),
            "{text}"
        );
    }
}
