use std::time::Duration;

use bytes::Bytes;
use percent_encoding::percent_decode_str;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_DISPOSITION,
    CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, LAST_MODIFIED, RANGE,
};
use reqwest::{Client, Response, StatusCode, Url};

use crate::{filename_from_content_disposition, NetError};

/// Settings for [`HttpClient`].
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Sent as the `User-Agent` header.
    pub user_agent: String,
    /// Use the system proxy settings (environment variables, and on Windows
    /// the system settings).
    pub use_system_proxy: bool,
    /// How long to wait for a connection to be made.
    pub connect_timeout: Duration,
    /// How long a connection may stay silent while receiving before it fails.
    pub read_timeout: Duration,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            user_agent: format!("Mohona/{}", env!("CARGO_PKG_VERSION")),
            use_system_proxy: true,
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(30),
        }
    }
}

/// What a server told us about a URL, before downloading it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// The address after following redirects. Download from this one.
    pub final_url: String,
    /// Size in bytes, if the server says.
    pub size: Option<u64>,
    /// Whether the server answers `Range` requests with partial content.
    pub accepts_ranges: bool,
    /// The `ETag` header, used later to detect that a file changed.
    pub etag: Option<String>,
    /// The `Last-Modified` header.
    pub last_modified: Option<String>,
    /// The `Content-Type` header.
    pub content_type: Option<String>,
    /// File name from `Content-Disposition`, or else from the URL path.
    /// This is the raw name from the server: sanitize it before using it.
    pub filename: Option<String>,
}

/// An HTTP(S) client. Cheap to clone; clones share connections.
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
}

/// The body of a successful response, read in chunks.
pub struct Body {
    response: Response,
}

impl Body {
    /// Length of the body in bytes, if the server said.
    pub fn content_length(&self) -> Option<u64> {
        self.response.content_length()
    }

    /// The next piece of the body, or `None` at the end.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, NetError> {
        Ok(self.response.chunk().await?)
    }
}

impl HttpClient {
    /// Build a client.
    pub fn new(config: &ClientConfig) -> Result<Self, NetError> {
        let mut headers = HeaderMap::new();
        // Ask for the file as stored. Compression would change sizes and
        // make byte ranges meaningless.
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let mut builder = Client::builder()
            .user_agent(config.user_agent.clone())
            .default_headers(headers)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout);
        if !config.use_system_proxy {
            builder = builder.no_proxy();
        }
        Ok(Self {
            client: builder.build()?,
        })
    }

    /// Find out size, range support, validators and file name for a URL.
    ///
    /// Sends `HEAD` first. If that fails, or does not prove range support,
    /// it asks for the first byte with `Range: bytes=0-0`, which also works
    /// on servers that reject `HEAD`.
    pub async fn probe(&self, url: &str) -> Result<Probe, NetError> {
        let url = parse_url(url)?;
        let mut from_head = None;

        if let Ok(response) = self.client.head(url.clone()).send().await {
            if response.status().is_success() {
                let mut probe = describe(&response);
                probe.size = header_text(&response, &CONTENT_LENGTH).and_then(|v| v.parse().ok());
                probe.accepts_ranges = header_text(&response, &ACCEPT_RANGES)
                    .is_some_and(|value| value.to_ascii_lowercase().contains("bytes"));
                if probe.accepts_ranges && probe.size.is_some() {
                    return Ok(probe);
                }
                from_head = Some(probe);
            }
        }

        match self
            .client
            .get(url.clone())
            .header(RANGE, "bytes=0-0")
            .send()
            .await
        {
            Ok(response) => probe_from_range_response(&response, from_head, &url),
            Err(error) => from_head.ok_or(NetError::Http(error)),
        }
    }

    /// Start a `GET` and return its body. Fails on any non-success status.
    pub async fn get(&self, url: &str) -> Result<Body, NetError> {
        let url = parse_url(url)?;
        let response = self.client.get(url.clone()).send().await?;
        if !response.status().is_success() {
            return Err(NetError::Status {
                status: response.status().as_u16(),
                url: url.to_string(),
            });
        }
        Ok(Body { response })
    }
}

fn parse_url(input: &str) -> Result<Url, NetError> {
    let url = Url::parse(input.trim())
        .map_err(|error| NetError::InvalidUrl(format!("{input} ({error})")))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(NetError::UnsupportedScheme(other.to_string())),
    }
}

fn header_text(response: &Response, name: &HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// The fields every kind of probe response fills in the same way.
fn describe(response: &Response) -> Probe {
    let filename = header_text(response, &CONTENT_DISPOSITION)
        .and_then(|value| filename_from_content_disposition(&value))
        .or_else(|| filename_from_url(response.url()));
    Probe {
        final_url: response.url().to_string(),
        size: None,
        accepts_ranges: false,
        etag: header_text(response, &ETAG),
        last_modified: header_text(response, &LAST_MODIFIED),
        content_type: header_text(response, &CONTENT_TYPE),
        filename,
    }
}

fn filename_from_url(url: &Url) -> Option<String> {
    let last = url.path_segments()?.next_back()?;
    let decoded = percent_decode_str(last).decode_utf8_lossy().into_owned();
    (!decoded.is_empty()).then_some(decoded)
}

/// `bytes 0-0/12345` gives `Some(12345)`; `bytes */0` gives `Some(0)`;
/// `bytes 0-0/*` gives `None`.
fn total_from_content_range(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse().ok()
}

fn probe_from_range_response(
    response: &Response,
    from_head: Option<Probe>,
    url: &Url,
) -> Result<Probe, NetError> {
    let mut probe = describe(response);
    match response.status() {
        StatusCode::PARTIAL_CONTENT => {
            probe.size =
                header_text(response, &CONTENT_RANGE).and_then(|v| total_from_content_range(&v));
            probe.accepts_ranges = true;
        }
        StatusCode::OK => {
            // The server ignored the Range header and would send everything.
            probe.size = header_text(response, &CONTENT_LENGTH).and_then(|v| v.parse().ok());
            probe.accepts_ranges = false;
        }
        StatusCode::RANGE_NOT_SATISFIABLE => {
            // Only an empty file cannot satisfy `bytes=0-0`.
            probe.size =
                header_text(response, &CONTENT_RANGE).and_then(|v| total_from_content_range(&v));
            probe.accepts_ranges = false;
        }
        other => {
            return from_head.ok_or(NetError::Status {
                status: other.as_u16(),
                url: url.to_string(),
            });
        }
    }
    Ok(probe)
}

#[cfg(test)]
mod tests {
    use super::total_from_content_range;

    #[test]
    fn reads_the_total_from_content_range() {
        assert_eq!(total_from_content_range("bytes 0-0/12345"), Some(12345));
        assert_eq!(total_from_content_range("bytes */0"), Some(0));
        assert_eq!(total_from_content_range("bytes 0-0/*"), None);
        assert_eq!(total_from_content_range("garbage"), None);
    }
}
