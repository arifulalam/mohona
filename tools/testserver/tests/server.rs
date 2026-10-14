//! Integration tests: start the server on a free port and talk to it over TCP.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use mohona_testserver::{bind, content_byte, serve, Config};

fn start() -> SocketAddr {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
            sender.send(listener.local_addr().unwrap()).unwrap();
            let _ = serve(
                listener,
                Config {
                    default_size: 1_000_000,
                },
            )
            .await;
        });
    });
    receiver.recv().unwrap()
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn parse(data: &[u8]) -> Response {
    let split = data
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response has no end of headers");
    let head = String::from_utf8_lossy(&data[..split]).to_string();
    let body = data[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    Response {
        status,
        headers,
        body,
    }
}

fn send(addr: SocketAddr, method: &str, path: &str, extra: &[&str]) -> Response {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n");
    for header in extra {
        request.push_str(header);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let mut data = Vec::new();
    let _ = stream.read_to_end(&mut data);
    parse(&data)
}

fn get(addr: SocketAddr, path: &str, extra: &[&str]) -> Response {
    send(addr, "GET", path, extra)
}

fn assert_content(body: &[u8], offset: u64) {
    for (i, byte) in body.iter().enumerate() {
        assert_eq!(*byte, content_byte(offset + i as u64), "byte {i} differs");
    }
}

#[test]
fn full_get_matches_generated_content() {
    let addr = start();
    let response = get(addr, "/file?size=100000", &[]);
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-length"), Some("100000"));
    assert_eq!(response.header("accept-ranges"), Some("bytes"));
    assert_eq!(response.body.len(), 100_000);
    assert_content(&response.body, 0);
}

#[test]
fn range_request_returns_206_with_the_right_bytes() {
    let addr = start();
    let response = get(addr, "/file?size=100000", &["Range: bytes=100-199"]);
    assert_eq!(response.status, 206);
    assert_eq!(
        response.header("content-range"),
        Some("bytes 100-199/100000")
    );
    assert_eq!(response.body.len(), 100);
    assert_content(&response.body, 100);
}

#[test]
fn open_ended_and_suffix_ranges_work() {
    let addr = start();
    let tail = get(addr, "/file?size=100000", &["Range: bytes=99990-"]);
    assert_eq!(tail.status, 206);
    assert_eq!(tail.body.len(), 10);
    assert_content(&tail.body, 99_990);

    let suffix = get(addr, "/file?size=100000", &["Range: bytes=-10"]);
    assert_eq!(suffix.status, 206);
    assert_eq!(suffix.body.len(), 10);
    assert_content(&suffix.body, 99_990);
}

#[test]
fn unsatisfiable_range_returns_416() {
    let addr = start();
    let response = get(addr, "/file?size=100000", &["Range: bytes=200000-"]);
    assert_eq!(response.status, 416);
    assert_eq!(response.header("content-range"), Some("bytes */100000"));
}

#[test]
fn norange_ignores_the_range_header() {
    let addr = start();
    let response = get(addr, "/file?size=1000&norange=1", &["Range: bytes=0-9"]);
    assert_eq!(response.status, 200);
    assert_eq!(response.body.len(), 1000);
    assert_eq!(response.header("accept-ranges"), None);
}

#[test]
fn if_range_with_a_different_etag_returns_the_whole_file() {
    let addr = start();
    let response = get(
        addr,
        "/file?size=1000",
        &["Range: bytes=0-9", "If-Range: \"something-else\""],
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.body.len(), 1000);
}

#[test]
fn if_range_with_the_current_etag_still_returns_a_range() {
    let addr = start();
    let response = get(
        addr,
        "/file?size=1000",
        &["Range: bytes=0-9", "If-Range: \"mohona-1000-1\""],
    );
    assert_eq!(response.status, 206);
    assert_eq!(response.body.len(), 10);
}

#[test]
fn head_has_headers_but_no_body() {
    let addr = start();
    let response = send(addr, "HEAD", "/file?size=1234", &[]);
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-length"), Some("1234"));
    assert!(response.body.is_empty());
}

#[test]
fn drop_after_cuts_the_body_short() {
    let addr = start();
    let response = get(addr, "/file?size=100000&drop_after=5000", &[]);
    assert_eq!(response.header("content-length"), Some("100000"));
    assert_eq!(response.body.len(), 5000);
    assert_content(&response.body, 0);
}

#[test]
fn random_drop_cuts_the_body_short() {
    let addr = start();
    let response = get(addr, "/file?size=100000&drop=random", &[]);
    assert!(!response.body.is_empty());
    assert!(response.body.len() < 100_000);
}

#[test]
fn limit_slows_the_transfer() {
    let addr = start();
    let started = Instant::now();
    let response = get(addr, "/file?size=50000&limit=100000", &[]);
    assert_eq!(response.body.len(), 50_000);
    assert!(started.elapsed() >= Duration::from_millis(400));
}

#[test]
fn basic_auth_is_required_on_auth() {
    let addr = start();
    let denied = get(addr, "/auth?size=10", &[]);
    assert_eq!(denied.status, 401);
    assert!(denied.header("www-authenticate").is_some());

    let allowed = get(
        addr,
        "/auth?size=10",
        &["Authorization: Basic bW9ob25hOnNlY3JldA=="],
    );
    assert_eq!(allowed.status, 200);
    assert_eq!(allowed.body.len(), 10);
}

#[test]
fn cookie_is_required_on_cookie() {
    let addr = start();
    assert_eq!(get(addr, "/cookie?size=10", &[]).status, 403);
    let login = get(addr, "/login", &[]);
    assert_eq!(login.header("set-cookie"), Some("session=ok; Path=/"));
    let allowed = get(addr, "/cookie?size=10", &["Cookie: session=ok"]);
    assert_eq!(allowed.status, 200);
}

#[test]
fn referer_is_required_on_referrer() {
    let addr = start();
    assert_eq!(get(addr, "/referrer?size=10", &[]).status, 403);
    let allowed = get(
        addr,
        "/referrer?size=10",
        &["Referer: https://example.test/page"],
    );
    assert_eq!(allowed.status, 200);
}

#[test]
fn redirect_points_to_file_and_keeps_the_query() {
    let addr = start();
    let response = get(addr, "/redirect?size=10", &[]);
    assert_eq!(response.status, 302);
    assert_eq!(response.header("location"), Some("/file?size=10"));
}

#[test]
fn unknown_paths_return_404() {
    let addr = start();
    assert_eq!(get(addr, "/nope", &[]).status, 404);
}

#[test]
fn keep_alive_serves_two_requests_on_one_connection() {
    let addr = start();
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .write_all(
            b"GET /file?size=10 HTTP/1.1\r\nHost: t\r\n\r\n\
              GET /file?size=10 HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut data = Vec::new();
    stream.read_to_end(&mut data).unwrap();
    let text = String::from_utf8_lossy(&data);
    assert_eq!(text.matches("HTTP/1.1 200 OK").count(), 2);
}
