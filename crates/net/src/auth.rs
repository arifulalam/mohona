//! HTTP authentication: Basic and Digest.
//!
//! Credentials are only sent in answer to a challenge from the server
//! (`401` with `WWW-Authenticate`), and only to the host that sent it. The
//! answer is remembered for the rest of the download, so each of the many
//! connections does not start with a refused request.

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A user name and password.
///
/// [`Debug`](fmt::Debug) never prints the password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    /// The user name.
    pub username: String,
    /// The password.
    pub password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<hidden>")
            .finish()
    }
}

impl Credentials {
    /// Parse `user:password` (the first colon separates them).
    pub fn parse(text: &str) -> Option<Self> {
        let (username, password) = text.split_once(':')?;
        (!username.is_empty()).then(|| Self {
            username: username.to_string(),
            password: password.to_string(),
        })
    }
}

/// Take `user:password@` out of an address, so the secret is not kept in
/// the download list or sent before the server asks for it. Returns the
/// address without it, and the credentials if there were any.
pub fn split_userinfo(url: &str) -> (String, Option<Credentials>) {
    let Ok(mut parsed) = reqwest::Url::parse(url.trim()) else {
        return (url.trim().to_string(), None);
    };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return (url.trim().to_string(), None);
    }
    let decode = |text: &str| {
        percent_encoding::percent_decode_str(text)
            .decode_utf8_lossy()
            .into_owned()
    };
    let credentials = Credentials {
        username: decode(parsed.username()),
        password: decode(parsed.password().unwrap_or("")),
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    (parsed.to_string(), Some(credentials))
}

/// How a server wants to be answered.
#[derive(Debug)]
pub(crate) enum Scheme {
    Basic,
    Digest(DigestChallenge),
}

/// The parts of a Digest challenge that stay the same between requests.
#[derive(Debug)]
pub(crate) struct DigestChallenge {
    realm: String,
    nonce: String,
    opaque: Option<String>,
    algorithm: Algorithm,
    session: bool,
    /// The server asked for `qop=auth`. Without it the older, simpler form
    /// of the answer is used.
    qop_auth: bool,
    /// Counts the uses of this nonce (`nc`).
    count: AtomicU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Algorithm {
    Md5,
    Sha256,
}

impl Algorithm {
    fn hash(self, data: &str) -> String {
        match self {
            Algorithm::Md5 => hex(&Md5::digest(data.as_bytes())),
            Algorithm::Sha256 => hex(&Sha256::digest(data.as_bytes())),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Algorithm::Md5 => "MD5",
            Algorithm::Sha256 => "SHA-256",
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Pick the best challenge we can answer from the `WWW-Authenticate`
/// header lines of a `401`: Digest (SHA-256 before MD5) before Basic.
pub(crate) fn choose(header_lines: &[String]) -> Option<Scheme> {
    let challenges: Vec<Challenge> = header_lines
        .iter()
        .flat_map(|l| parse_challenges(l))
        .collect();
    let mut best: Option<(u8, DigestChallenge)> = None;
    for challenge in &challenges {
        if !challenge.scheme.eq_ignore_ascii_case("digest") {
            continue;
        }
        if let Some((rank, digest)) = digest_from(challenge) {
            if best.as_ref().is_none_or(|(r, _)| rank > *r) {
                best = Some((rank, digest));
            }
        }
    }
    if let Some((_, digest)) = best {
        return Some(Scheme::Digest(digest));
    }
    challenges
        .iter()
        .any(|c| c.scheme.eq_ignore_ascii_case("basic"))
        .then_some(Scheme::Basic)
}

fn digest_from(challenge: &Challenge) -> Option<(u8, DigestChallenge)> {
    let get = |name: &str| {
        challenge
            .params
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let algorithm_text = get("algorithm").unwrap_or("MD5").to_ascii_uppercase();
    let (base, session) = match algorithm_text.strip_suffix("-SESS") {
        Some(base) => (base, true),
        None => (algorithm_text.as_str(), false),
    };
    let (algorithm, rank) = match base {
        "MD5" => (Algorithm::Md5, 1),
        "SHA-256" => (Algorithm::Sha256, 2),
        _ => return None,
    };
    let qop_auth = match get("qop") {
        None => false,
        Some(list) => {
            if !list
                .split(',')
                .any(|q| q.trim().eq_ignore_ascii_case("auth"))
            {
                // Only `auth-int` (a hash of the body) is offered: not supported.
                return None;
            }
            true
        }
    };
    Some((
        rank,
        DigestChallenge {
            realm: get("realm")?.to_string(),
            nonce: get("nonce")?.to_string(),
            opaque: get("opaque").map(str::to_string),
            algorithm,
            session,
            qop_auth,
            count: AtomicU32::new(0),
        },
    ))
}

impl Scheme {
    /// The `Authorization` header value for a request.
    pub(crate) fn authorization(
        &self,
        credentials: &Credentials,
        method: &str,
        uri: &str,
    ) -> String {
        match self {
            Scheme::Basic => format!(
                "Basic {}",
                STANDARD.encode(format!("{}:{}", credentials.username, credentials.password))
            ),
            Scheme::Digest(d) => d.authorization(credentials, method, uri, &random_text()),
        }
    }
}

impl DigestChallenge {
    fn authorization(&self, c: &Credentials, method: &str, uri: &str, cnonce: &str) -> String {
        let h = |s: &str| self.algorithm.hash(s);
        let mut ha1 = h(&format!("{}:{}:{}", c.username, self.realm, c.password));
        if self.session {
            ha1 = h(&format!("{ha1}:{}:{cnonce}", self.nonce));
        }
        let ha2 = h(&format!("{method}:{uri}"));
        let nc = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        let nc = format!("{nc:08x}");
        let response = if self.qop_auth {
            h(&format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", self.nonce))
        } else {
            h(&format!("{ha1}:{}:{ha2}", self.nonce))
        };
        let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let mut out = format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", algorithm={}{}, response=\"{}\"",
            quote(&c.username),
            quote(&self.realm),
            quote(&self.nonce),
            quote(uri),
            self.algorithm.name(),
            if self.session { "-sess" } else { "" },
            response
        );
        if let Some(opaque) = &self.opaque {
            out.push_str(&format!(", opaque=\"{}\"", quote(opaque)));
        }
        if self.qop_auth {
            out.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
        }
        out
    }
}

fn random_text() -> String {
    let mut bytes = [0u8; 12];
    if getrandom::fill(&mut bytes).is_err() {
        // No system randomness: fall back to the clock. A cnonce only needs
        // to differ between requests; it is not a secret.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes.copy_from_slice(&nanos.to_le_bytes()[..12]);
    }
    hex(&bytes)
}

/// One challenge from a `WWW-Authenticate` header.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub scheme: String,
    pub params: Vec<(String, String)>,
}

/// Split a header value such as
/// `Basic realm="x", Digest realm="y", nonce="n", qop="auth,auth-int"`
/// into its challenges.
pub(crate) fn parse_challenges(value: &str) -> Vec<Challenge> {
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    let mut out: Vec<Challenge> = Vec::new();
    let skip = |i: &mut usize, set: &dyn Fn(char) -> bool| {
        while *i < chars.len() && set(chars[*i]) {
            *i += 1;
        }
    };
    loop {
        skip(&mut i, &|c| c == ',' || c.is_whitespace());
        if i >= chars.len() {
            break;
        }
        let start = i;
        skip(&mut i, &|c| c != '=' && c != ',' && !c.is_whitespace());
        let token: String = chars[start..i].iter().collect();
        let after_token = i;
        skip(&mut i, &|c| c.is_whitespace());
        if i < chars.len() && chars[i] == '=' && !out.is_empty() {
            // `name=value` belongs to the challenge before it.
            i += 1;
            skip(&mut i, &|c| c.is_whitespace());
            let text = if i < chars.len() && chars[i] == '"' {
                i += 1;
                let mut text = String::new();
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
                i += 1; // closing quote
                text
            } else {
                let s = i;
                skip(&mut i, &|c| c != ',' && !c.is_whitespace());
                chars[s..i].iter().collect()
            };
            if let Some(last) = out.last_mut() {
                last.params.push((token, text));
            }
        } else {
            // A new challenge starts with its scheme name.
            i = after_token;
            out.push(Challenge {
                scheme: token,
                params: Vec::new(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_one_or_several_challenges() {
        let c = parse_challenges(
            r#"Basic realm="a b", Digest realm="r, with comma", nonce="n\"q", qop="auth,auth-int", algorithm=SHA-256"#,
        );
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].scheme, "Basic");
        assert_eq!(c[0].params, [("realm".into(), "a b".into())]);
        assert_eq!(c[1].scheme, "Digest");
        let p = &c[1].params;
        assert_eq!(p[0], ("realm".into(), "r, with comma".into()));
        assert_eq!(p[1], ("nonce".into(), "n\"q".into()));
        assert_eq!(p[2], ("qop".into(), "auth,auth-int".into()));
        assert_eq!(p[3], ("algorithm".into(), "SHA-256".into()));
    }

    #[test]
    fn prefers_sha256_digest_over_md5_over_basic() {
        let both = lines(&[
            r#"Basic realm="x""#,
            r#"Digest realm="x", nonce="1", algorithm=MD5, qop="auth""#,
            r#"Digest realm="x", nonce="2", algorithm=SHA-256, qop="auth""#,
        ]);
        let Some(Scheme::Digest(d)) = choose(&both) else {
            panic!("expected digest");
        };
        assert_eq!(d.algorithm, Algorithm::Sha256);
        assert_eq!(d.nonce, "2");
        assert!(matches!(
            choose(&lines(&[r#"Basic realm="x""#])),
            Some(Scheme::Basic)
        ));
        assert!(choose(&lines(&[r#"Negotiate"#])).is_none());
        // Only auth-int or an unknown algorithm: cannot answer, falls to Basic.
        let odd = lines(&[
            r#"Digest realm="x", nonce="1", qop="auth-int""#,
            r#"Digest realm="x", nonce="1", algorithm=SHA-512-256"#,
            r#"Basic realm="x""#,
        ]);
        assert!(matches!(choose(&odd), Some(Scheme::Basic)));
    }

    #[test]
    fn basic_header_is_standard() {
        let c = Credentials::parse("mohona:secret").unwrap();
        assert_eq!(
            Scheme::Basic.authorization(&c, "GET", "/"),
            "Basic bW9ob25hOnNlY3JldA=="
        );
    }

    /// The worked example from RFC 2617 section 3.5 (MD5, qop=auth).
    #[test]
    fn digest_matches_the_rfc_example() {
        let challenge = parse_challenges(
            r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#,
        );
        let (_, d) = digest_from(&challenge[0]).unwrap();
        let c = Credentials::parse("Mufasa:Circle Of Life").unwrap();
        let header = d.authorization(&c, "GET", "/dir/index.html", "0a4f113b");
        assert!(
            header.contains(r#"response="6629fae49393a05397450978507c4ef1""#),
            "{header}"
        );
        assert!(header.contains("nc=00000001"));
        assert!(header.contains(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#));
        // The next use of the same nonce counts up.
        let again = d.authorization(&c, "GET", "/dir/index.html", "0a4f113b");
        assert!(again.contains("nc=00000002"));
    }

    #[test]
    fn digest_without_qop_uses_the_older_form() {
        let challenge = parse_challenges(r#"Digest realm="r", nonce="n""#);
        let (_, d) = digest_from(&challenge[0]).unwrap();
        let c = Credentials::parse("u:p").unwrap();
        let header = d.authorization(&c, "GET", "/x", "unused");
        assert!(
            !header.contains("nc=") && !header.contains("cnonce"),
            "{header}"
        );
        let ha1 = Algorithm::Md5.hash("u:r:p");
        let ha2 = Algorithm::Md5.hash("GET:/x");
        let expected = Algorithm::Md5.hash(&format!("{ha1}:n:{ha2}"));
        assert!(
            header.contains(&format!("response=\"{expected}\"")),
            "{header}"
        );
    }

    #[test]
    fn userinfo_is_split_from_the_address() {
        let (url, c) = split_userinfo("https://us%40er:p%3Aw@example.org/a/b?x=1");
        assert_eq!(url, "https://example.org/a/b?x=1");
        let c = c.unwrap();
        assert_eq!((c.username.as_str(), c.password.as_str()), ("us@er", "p:w"));
        assert_eq!(
            split_userinfo("https://example.org/plain"),
            ("https://example.org/plain".to_string(), None)
        );
        assert_eq!(split_userinfo("not a url").1, None);
    }

    #[test]
    fn credentials_debug_hides_the_password() {
        let c = Credentials::parse("me:hunter2").unwrap();
        assert!(!format!("{c:?}").contains("hunter2"));
        assert!(Credentials::parse(":nouser").is_none());
        assert!(Credentials::parse("nocolon").is_none());
    }
}
