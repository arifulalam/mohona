//! HTTP Digest authentication for the test server, strict enough to catch
//! clients that get it wrong: the answer is recomputed from scratch, the
//! nonce must be one this server issued, and `nc` must keep rising.

use std::collections::HashMap;
use std::sync::Mutex;

use md5::Md5;
use sha2::{Digest, Sha256};

pub const USER: &str = "mohona";
pub const PASSWORD: &str = "secret";
pub const REALM: &str = "mohona";

#[derive(Default)]
pub struct State {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    next_nonce: u64,
    /// nonce -> (highest nc seen, answers accepted)
    nonces: HashMap<String, (u32, usize)>,
}

#[derive(Clone, Copy)]
pub struct Options {
    pub sha256: bool,
    /// `qop=auth` (the normal form) or none (the old RFC 2069 form).
    pub qop: bool,
    /// A nonce stops working after this many accepted answers (0 = never).
    pub expire: usize,
}

fn hash(sha256: bool, text: &str) -> String {
    let bytes: Vec<u8> = if sha256 {
        Sha256::digest(text.as_bytes()).to_vec()
    } else {
        Md5::digest(text.as_bytes()).to_vec()
    };
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Split `Digest a="b", c=d` into pairs (values may be quoted and contain
/// commas).
fn parse(header: &str) -> Option<HashMap<String, String>> {
    let rest = header.trim().strip_prefix("Digest ")?;
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    let mut map = HashMap::new();
    while i < chars.len() {
        while i < chars.len() && (chars[i] == ',' || chars[i].is_whitespace()) {
            i += 1;
        }
        let start = i;
        while i < chars.len() && chars[i] != '=' {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let name: String = chars[start..i].iter().collect();
        i += 1;
        let mut value = String::new();
        if i < chars.len() && chars[i] == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                value.push(chars[i]);
                i += 1;
            }
            i += 1;
        } else {
            while i < chars.len() && chars[i] != ',' {
                value.push(chars[i]);
                i += 1;
            }
        }
        map.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    Some(map)
}

impl State {
    /// A `WWW-Authenticate` value with a fresh nonce.
    pub fn challenge(&self, options: Options, stale: bool) -> String {
        let nonce = {
            let mut inner = self.inner.lock().unwrap();
            inner.next_nonce += 1;
            let nonce = format!("nonce-{:08x}", inner.next_nonce);
            inner.nonces.insert(nonce.clone(), (0, 0));
            nonce
        };
        let mut text = format!(
            "Digest realm=\"{REALM}\", nonce=\"{nonce}\", opaque=\"op-{nonce}\", algorithm={}",
            if options.sha256 { "SHA-256" } else { "MD5" }
        );
        if options.qop {
            text.push_str(", qop=\"auth\"");
        }
        if stale {
            text.push_str(", stale=true");
        }
        text
    }

    /// `Ok` if the `Authorization` header is a correct, fresh answer.
    /// `Err(stale)` otherwise: `stale` says the nonce was right but used up.
    pub fn verify(
        &self,
        header: Option<&str>,
        method: &str,
        target: &str,
        options: Options,
    ) -> Result<(), bool> {
        let fields = header.and_then(parse).ok_or(false)?;
        let get = |name: &str| fields.get(name).map(String::as_str).unwrap_or("");
        let algorithm = get("algorithm").to_ascii_uppercase();
        let expected_algorithm = if options.sha256 { "SHA-256" } else { "MD5" };
        if get("username") != USER
            || get("realm") != REALM
            || get("uri") != target
            || (!algorithm.is_empty() && algorithm != expected_algorithm)
            || get("opaque") != format!("op-{}", get("nonce"))
        {
            return Err(false);
        }
        let ha1 = hash(options.sha256, &format!("{USER}:{REALM}:{PASSWORD}"));
        let ha2 = hash(options.sha256, &format!("{method}:{target}"));
        let nonce = get("nonce");
        let (expected, nc) = if options.qop {
            if get("qop") != "auth" || get("cnonce").is_empty() {
                return Err(false);
            }
            let nc = u32::from_str_radix(get("nc"), 16).map_err(|_| false)?;
            let text = format!("{ha1}:{nonce}:{}:{}:auth:{ha2}", get("nc"), get("cnonce"));
            (hash(options.sha256, &text), Some(nc))
        } else {
            (hash(options.sha256, &format!("{ha1}:{nonce}:{ha2}")), None)
        };
        if get("response") != expected {
            return Err(false);
        }
        let mut inner = self.inner.lock().unwrap();
        let Some((seen_nc, accepted)) = inner.nonces.get_mut(nonce) else {
            return Err(false);
        };
        if options.expire > 0 && *accepted >= options.expire {
            return Err(true);
        }
        if let Some(nc) = nc {
            if nc <= *seen_nc {
                // A replay (or a client that does not count): not accepted.
                return Err(false);
            }
            *seen_nc = nc;
        }
        *accepted += 1;
        Ok(())
    }
}
