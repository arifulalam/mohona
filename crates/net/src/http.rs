use std::time::Duration;

use bytes::Bytes;
use percent_encoding::percent_decode_str;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_DISPOSITION,
    CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE, RETRY_AFTER,
};
use reqwest::{Client, Response, StatusCode, Url};

use std::sync::{Arc, Mutex};

use reqwest::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Method, RequestBuilder};

use crate::auth::{self, Scheme};
use crate::{filename_from_content_disposition, Credentials, NetError, RequestContext};

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

/// The origin that was logged in to, and how.
type Answered = Arc<Mutex<Option<(String, Arc<Scheme>)>>>;

/// An HTTP(S) client. Cheap to clone; clones share connections.
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    extra: HeaderMap,
    credentials: Option<Credentials>,
    /// How the server wanted to be answered, for the rest of the download.
    answered: Answered,
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
            extra: HeaderMap::new(),
            credentials: None,
            answered: Arc::default(),
        })
    }

    /// A client that sends `context` with every request. It shares
    /// connections with this one. Fails if the context holds a header that
    /// cannot be sent.
    pub fn with_context(&self, context: &RequestContext) -> Result<Self, NetError> {
        Ok(Self {
            client: self.client.clone(),
            extra: context.header_map()?,
            credentials: context.credentials.clone(),
            answered: Arc::default(),
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

        if let Ok(response) = self.send(Method::HEAD, &url, |r| r).await {
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
            .send(Method::GET, &url, |r| r.header(RANGE, "bytes=0-0"))
            .await
        {
            Ok(response) => probe_from_range_response(&response, from_head, &url),
            Err(error) => from_head.ok_or(NetError::Http(error)),
        }
    }

    /// Start a `GET` and return its body. Fails on any non-success status.
    pub async fn get(&self, url: &str) -> Result<Body, NetError> {
        let url = parse_url(url)?;
        let response = self.send(Method::GET, &url, |r| r).await?;
        if !response.status().is_success() {
            return Err(status_error(&response, &url));
        }
        Ok(Body { response })
    }

    /// Start a `GET` for bytes `first..=last` and return its body.
    ///
    /// The answer must be `206 Partial Content` starting at `first`; anything
    /// else (for example the whole file, because the server ignores ranges or
    /// the file changed) is an error, so a wrong part is never written into
    /// the file. If `etag` is given it is sent as `If-Range`, which makes a
    /// server whose file changed answer 200 instead of the new bytes.
    pub async fn get_range(
        &self,
        url: &str,
        first: u64,
        last: u64,
        etag: Option<&str>,
    ) -> Result<Body, NetError> {
        let url = parse_url(url)?;
        let range = format!("bytes={first}-{last}");
        // Weak validators ("W/...") are not allowed in `If-Range`.
        let if_range = etag.filter(|etag| !etag.starts_with("W/"));
        let response = self
            .send(Method::GET, &url, |mut r| {
                r = r.header(RANGE, &range);
                if let Some(etag) = if_range {
                    r = r.header(IF_RANGE, etag);
                }
                r
            })
            .await?;
        let status = response.status();
        if status == StatusCode::PARTIAL_CONTENT {
            let start = header_text(&response, &CONTENT_RANGE)
                .and_then(|value| first_byte_from_content_range(&value));
            if start != Some(first) {
                return Err(NetError::WrongRange(format!(
                    "asked for bytes from {first}, got {:?}",
                    header_text(&response, &CONTENT_RANGE)
                )));
            }
            Ok(Body { response })
        } else if status.is_success() {
            Err(NetError::WrongRange(format!(
                "asked for bytes {first}-{last}, got status {}",
                status.as_u16()
            )))
        } else {
            Err(status_error(&response, &url))
        }
    }
}

impl HttpClient {
    /// Send a request. If the server answers `401` and credentials were
    /// given, answer its challenge (Digest preferred, else Basic) and send
    /// again, once. What worked is remembered, so later requests carry the
    /// answer from the start.
    ///
    /// Credentials go only to the host the request was addressed to: if a
    /// redirect led somewhere else, a challenge from there is not answered.
    async fn send(
        &self,
        method: Method,
        url: &Url,
        shape: impl Fn(RequestBuilder) -> RequestBuilder,
    ) -> Result<Response, reqwest::Error> {
        let build = |target: &Url, authorization: Option<String>| {
            let mut request = shape(
                self.client
                    .request(method.clone(), target.clone())
                    .headers(self.extra.clone()),
            );
            if let Some(mut text) = authorization.and_then(|t| HeaderValue::from_str(&t).ok()) {
                text.set_sensitive(true);
                request = request.header(AUTHORIZATION, text);
            }
            request
        };
        let Some(credentials) = &self.credentials else {
            return build(url, None).send().await;
        };
        let origin = origin_of(url);
        let remembered = self
            .answered
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .filter(|(known, _)| *known == origin)
            .map(|(_, scheme)| scheme);
        let first = remembered
            .as_ref()
            .map(|scheme| scheme.authorization(credentials, method.as_str(), &request_uri(url)));
        let response = build(url, first).send().await?;
        if response.status() != StatusCode::UNAUTHORIZED || origin_of(response.url()) != origin {
            return Ok(response);
        }
        let lines: Vec<String> = response
            .headers()
            .get_all(WWW_AUTHENTICATE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(str::to_string))
            .collect();
        let Some(scheme) = auth::choose(&lines) else {
            return Ok(response);
        };
        // Answer at the address the challenge came from (after redirects).
        let target = response.url().clone();
        let scheme = Arc::new(scheme);
        let header = scheme.authorization(credentials, method.as_str(), &request_uri(&target));
        let second = build(&target, Some(header)).send().await?;
        if second.status() != StatusCode::UNAUTHORIZED {
            if let Ok(mut slot) = self.answered.lock() {
                *slot = Some((origin, scheme));
            }
        }
        Ok(second)
    }
}

/// Scheme, host and port: what "the same server" means for credentials.
fn origin_of(url: &Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or("").to_ascii_lowercase(),
        url.port_or_known_default().unwrap_or(0)
    )
}

/// The path and query as they appear in the request line (the Digest `uri`).
fn request_uri(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    }
}

fn status_error(response: &Response, url: &Url) -> NetError {
    NetError::Status {
        status: response.status().as_u16(),
        url: url.to_string(),
        retry_after: header_text(response, &RETRY_AFTER).and_then(|v| parse_retry_after(&v)),
    }
}

/// `Retry-After: 120` gives 120 seconds. The date form is not supported and
/// gives `None`, which callers treat as "use the normal backoff".
fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
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

/// `bytes 100-199/1000` gives `Some(100)`.
fn first_byte_from_content_range(value: &str) -> Option<u64> {
    let range = value.trim().strip_prefix("bytes")?.trim_start();
    range.split_once('-')?.0.trim().parse().ok()
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
        _ => {
            return from_head.ok_or_else(|| status_error(response, url));
        }
    }
    Ok(probe)
}

#[cfg(test)]
mod tests {
    use super::{first_byte_from_content_range, parse_retry_after, total_from_content_range};
    use std::time::Duration;

    #[test]
    fn reads_retry_after_seconds() {
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after(" 3 "), Some(Duration::from_secs(3)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    }

    #[test]
    fn reads_the_first_byte_from_content_range() {
        assert_eq!(
            first_byte_from_content_range("bytes 100-199/1000"),
            Some(100)
        );
        assert_eq!(first_byte_from_content_range("bytes 0-0/*"), Some(0));
        assert_eq!(first_byte_from_content_range("bytes */1000"), None);
        assert_eq!(first_byte_from_content_range("garbage"), None);
    }

    #[test]
    fn reads_the_total_from_content_range() {
        assert_eq!(total_from_content_range("bytes 0-0/12345"), Some(12345));
        assert_eq!(total_from_content_range("bytes */0"), Some(0));
        assert_eq!(total_from_content_range("bytes 0-0/*"), None);
        assert_eq!(total_from_content_range("garbage"), None);
    }
}
