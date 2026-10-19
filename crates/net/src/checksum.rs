//! Checksums: what a file should hash to, and how to compute it.

use std::fmt;
use std::io::Read;
use std::path::Path;

use base64::Engine;
use md5::Md5;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use crate::NetError;

/// A hash algorithm, from weakest to strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Algorithm {
    /// MD5 (fine for catching damage, not for catching tampering).
    Md5,
    /// SHA-1.
    Sha1,
    /// SHA-256.
    Sha256,
    /// SHA-512.
    Sha512,
}

impl Algorithm {
    /// The short name: `md5`, `sha1`, `sha256`, `sha512`.
    pub fn name(self) -> &'static str {
        match self {
            Algorithm::Md5 => "md5",
            Algorithm::Sha1 => "sha1",
            Algorithm::Sha256 => "sha256",
            Algorithm::Sha512 => "sha512",
        }
    }

    /// Read a name such as `sha256`, `SHA-256` or `md5`.
    pub fn parse(name: &str) -> Option<Self> {
        let name: String = name
            .trim()
            .chars()
            .filter(|c| *c != '-' && *c != '_')
            .collect::<String>()
            .to_ascii_lowercase();
        match name.as_str() {
            "md5" => Some(Algorithm::Md5),
            "sha1" => Some(Algorithm::Sha1),
            "sha256" => Some(Algorithm::Sha256),
            "sha512" => Some(Algorithm::Sha512),
            _ => None,
        }
    }

    fn hex_len(self) -> usize {
        match self {
            Algorithm::Md5 => 32,
            Algorithm::Sha1 => 40,
            Algorithm::Sha256 => 64,
            Algorithm::Sha512 => 128,
        }
    }

    fn from_hex_len(len: usize) -> Option<Self> {
        [
            Algorithm::Md5,
            Algorithm::Sha1,
            Algorithm::Sha256,
            Algorithm::Sha512,
        ]
        .into_iter()
        .find(|a| a.hex_len() == len)
    }
}

/// What a file should hash to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checksum {
    /// How it was hashed.
    pub algorithm: Algorithm,
    /// The hash as lowercase hex.
    pub hex: String,
}

impl fmt::Display for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm.name(), self.hex)
    }
}

impl Checksum {
    /// Read `sha256:HEX`, `sha256=HEX` or just `HEX` (the algorithm is then
    /// told from the length: 32 md5, 40 sha1, 64 sha256, 128 sha512).
    pub fn parse(text: &str) -> Result<Self, NetError> {
        let bad = |why: &str| NetError::InvalidChecksum(format!("{text:?}: {why}"));
        let text = text.trim();
        let (named, hex) =
            match text.split_once([':', '=']) {
                Some((name, hex)) => (
                    Some(Algorithm::parse(name).ok_or_else(|| {
                        bad("unknown algorithm (use md5, sha1, sha256 or sha512)")
                    })?),
                    hex.trim(),
                ),
                None => (None, text),
            };
        if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(bad("a checksum is made of the digits 0-9 and letters a-f"));
        }
        let algorithm = match named {
            Some(algorithm) => algorithm,
            None => Algorithm::from_hex_len(hex.len()).ok_or_else(|| {
                bad("cannot tell the algorithm from this length; write sha256:...")
            })?,
        };
        if hex.len() != algorithm.hex_len() {
            return Err(bad(&format!(
                "{} needs {} characters, this has {}",
                algorithm.name(),
                algorithm.hex_len(),
                hex.len()
            )));
        }
        Ok(Self {
            algorithm,
            hex: hex.to_ascii_lowercase(),
        })
    }

    /// From the base64 form used in HTTP headers.
    fn from_base64(algorithm: Algorithm, text: &str) -> Option<Self> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .ok()?;
        (bytes.len() * 2 == algorithm.hex_len()).then(|| Self {
            algorithm,
            hex: to_hex(&bytes),
        })
    }

    /// The `Content-MD5` header (a base64 MD5 of the whole body).
    pub fn from_content_md5(value: &str) -> Option<Self> {
        Self::from_base64(Algorithm::Md5, value)
    }

    /// The `Digest` header of RFC 3230 (`sha-256=BASE64, md5=BASE64`) or
    /// `Repr-Digest` of RFC 9530 (`sha-256=:BASE64:`). Unknown algorithms
    /// are skipped.
    pub fn from_digest_header(value: &str) -> Vec<Self> {
        value
            .split(',')
            .filter_map(|part| {
                let (name, data) = part.split_once('=')?;
                let algorithm = Algorithm::parse(name)?;
                Self::from_base64(algorithm, data.trim().trim_matches(':'))
            })
            .collect()
    }

    /// Find the hash for `filename` in the text of a checksum file. Understands
    /// `HEX  name`, `HEX *name`, `ALGO (name) = HEX`, and a lone `HEX`.
    /// The algorithm is told from the file's extension or the hash length.
    pub fn from_checksum_file(text: &str, filename: &str, algorithm: Algorithm) -> Option<Self> {
        let mut only: Option<Self> = None;
        let mut lines = 0;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            lines += 1;
            let (hex, name) = if let Some((left, hex)) = line.rsplit_once(" = ") {
                // BSD style: SHA256 (name) = HEX
                let name = left
                    .split_once('(')
                    .and_then(|(_, rest)| rest.strip_suffix(')'));
                (hex.trim(), name.unwrap_or(""))
            } else {
                let (hex, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
                (hex, rest.trim().trim_start_matches('*'))
            };
            let Ok(found) = Self::parse(&format!("{}:{hex}", algorithm.name())) else {
                continue;
            };
            let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
            if !name.is_empty() && base == filename {
                return Some(found);
            }
            if only.is_none() {
                only = Some(found);
            }
        }
        // A file that names other files and not this one says nothing about it.
        if lines == 1 {
            let line = text.trim();
            let names_other =
                line.split_once(char::is_whitespace).is_some() && !line.contains(filename);
            return if names_other { None } else { only };
        }
        None
    }
}

impl Serialize for Checksum {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Checksum {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Checksum::parse(&text).map_err(serde::de::Error::custom)
    }
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Hash a file (blocking: run it where blocking is fine).
pub fn hash_file(path: &Path, algorithm: Algorithm) -> std::io::Result<String> {
    fn run<D: Digest>(path: &Path) -> std::io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = D::new();
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(to_hex(&hasher.finalize()))
    }
    match algorithm {
        Algorithm::Md5 => run::<Md5>(path),
        Algorithm::Sha1 => run::<Sha1>(path),
        Algorithm::Sha256 => run::<Sha256>(path),
        Algorithm::Sha512 => run::<Sha512>(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const EMPTY_MD5: &str = "d41d8cd98f00b204e9800998ecf8427e";

    #[test]
    fn reads_checksums_in_the_ways_people_write_them() {
        let a = Checksum::parse(EMPTY_SHA256).unwrap();
        assert_eq!(a.algorithm, Algorithm::Sha256);
        assert_eq!(
            Checksum::parse(&format!("SHA256:{}", EMPTY_SHA256.to_uppercase())).unwrap(),
            a
        );
        assert_eq!(
            Checksum::parse(&format!("sha-256={EMPTY_SHA256}")).unwrap(),
            a
        );
        assert_eq!(
            Checksum::parse(EMPTY_MD5).unwrap().algorithm,
            Algorithm::Md5
        );
        assert_eq!(a.to_string(), format!("sha256:{EMPTY_SHA256}"));
    }

    #[test]
    fn refuses_bad_checksums_with_a_reason() {
        for bad in ["", "xyz", "abc123", "sha256:abc", "foo:abcd", "sha256:zz"] {
            let err = Checksum::parse(bad).unwrap_err().to_string();
            assert!(err.contains("invalid checksum"), "{bad}: {err}");
        }
    }

    #[test]
    fn hashes_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(hash_file(&path, Algorithm::Sha256).unwrap(), EMPTY_SHA256);
        assert_eq!(hash_file(&path, Algorithm::Md5).unwrap(), EMPTY_MD5);
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            hash_file(&path, Algorithm::Sha1).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn reads_http_digest_headers() {
        // MD5 of "" is 1B2M2Y8AsgTpgAmY7PhCfg== in base64.
        let md5 = Checksum::from_content_md5("1B2M2Y8AsgTpgAmY7PhCfg==").unwrap();
        assert_eq!(md5.hex, EMPTY_MD5);
        let found = Checksum::from_digest_header(
            "unknown=AAAA, sha-256=47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=, md5=1B2M2Y8AsgTpgAmY7PhCfg==",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].algorithm, Algorithm::Sha256);
        let repr =
            Checksum::from_digest_header("sha-256=:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=:");
        assert_eq!(repr[0].hex, EMPTY_SHA256);
        assert!(Checksum::from_content_md5("not base64!").is_none());
        assert!(Checksum::from_content_md5("AAAA").is_none());
    }

    #[test]
    fn reads_checksum_files() {
        let h = EMPTY_SHA256;
        let s = Algorithm::Sha256;
        let get =
            |text: &str, name: &str| Checksum::from_checksum_file(text, name, s).map(|c| c.hex);
        assert_eq!(get(&format!("{h}\n"), "a.iso").as_deref(), Some(h));
        assert_eq!(get(&format!("{h}  a.iso\n"), "a.iso").as_deref(), Some(h));
        assert_eq!(get(&format!("{h} *a.iso\n"), "a.iso").as_deref(), Some(h));
        assert_eq!(
            get(&format!("SHA256 (a.iso) = {h}\n"), "a.iso").as_deref(),
            Some(h)
        );
        let other = "1".repeat(64);
        let many = format!("{other}  b.iso\n{h}  dir/a.iso\n");
        assert_eq!(get(&many, "a.iso").as_deref(), Some(h));
        // Lists that do not mention the file say nothing about it.
        assert_eq!(get(&many, "c.iso"), None);
        assert_eq!(get(&format!("{other}  b.iso\n"), "a.iso"), None);
        assert_eq!(get("<html>not found</html>", "a.iso"), None);
    }
}
