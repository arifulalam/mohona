//! The grabber against a small web site served on this computer.

use std::collections::HashMap;
use std::time::Duration;

use mohona_core::CancelToken;
use mohona_grabber::{grab, Options};
use mohona_net::{ClientConfig, HttpClient};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve(pages: HashMap<&'static str, (&'static str, String)>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let pages = pages.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let n = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..n]).to_string();
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .split('?')
                    .next()
                    .unwrap_or("/")
                    .to_string();
                let answer = match pages.get(path.as_str()) {
                    Some((kind, body)) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string(),
                };
                let _ = socket.write_all(answer.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    addr
}

fn site() -> HashMap<&'static str, (&'static str, String)> {
    let html = "text/html; charset=utf-8";
    HashMap::from([
        (
            "/robots.txt",
            ("text/plain", "User-agent: *\nDisallow: /private/\n".to_string()),
        ),
        (
            "/index.html",
            (
                html,
                r#"<a href="a.pdf">A</a> <a href="/sub/page.html">sub</a>
                   <a href="/private/secret.html">secret</a> <img src="/p.jpg">
                   <a href="https://elsewhere.invalid/page.html">out</a>
                   <a href="https://elsewhere.invalid/z.zip">z</a>
                   <a href="/style.css">css</a> <a href="/data">data</a>"#
                    .to_string(),
            ),
        ),
        (
            "/sub/page.html",
            (
                html,
                r#"<a href="b.zip">B</a> <a href="/deeper/more.html">more</a> <a href="/index.html">home</a>"#
                    .to_string(),
            ),
        ),
        (
            "/deeper/more.html",
            (html, r#"<a href="c.mp4">C</a>"#.to_string()),
        ),
        (
            "/private/secret.html",
            (html, r#"<a href="hidden.zip">H</a>"#.to_string()),
        ),
        // No ending, but not a page: its content type says it is a file.
        ("/data", ("application/octet-stream", "0123456789".to_string())),
    ])
}

fn options(depth: u32) -> Options {
    Options {
        depth,
        delay: Duration::from_millis(1),
        ..Options::default()
    }
}

#[tokio::test]
async fn files_are_listed_to_the_depth_asked_and_the_site_stays_polite() {
    let addr = serve(site()).await;
    let client = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    let start = format!("http://{addr}/index.html");
    let names = |g: &mohona_grabber::Grabbed| {
        let mut v: Vec<String> = g.files.iter().map(|f| f.name.clone()).collect();
        v.sort();
        v
    };

    // Depth 0: only the first page's links. The picture, the pdf and the file elsewhere; no css.
    let g = grab(&client, &start, &options(0), &CancelToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(names(&g), vec!["a.pdf", "p.jpg", "z.zip"]);
    assert_eq!(g.pages, 1);

    // Depth 1 reads the sub page (and /data, found to be a file), not the other site, and leaves
    // /private/ alone as the site asks.
    let g = grab(&client, &start, &options(1), &CancelToken::new(), |_| {})
        .await
        .unwrap();
    assert_eq!(names(&g), vec!["a.pdf", "b.zip", "data", "p.jpg", "z.zip"]);
    assert_eq!(g.skipped_by_robots, 1);
    let b = g.files.iter().find(|f| f.name == "b.zip").unwrap();
    assert!(b.page.ends_with("/sub/page.html"));

    // Depth 2 reaches the deeper page; robots.txt switched off reaches the private one.
    let g = grab(&client, &start, &options(2), &CancelToken::new(), |_| {})
        .await
        .unwrap();
    assert!(names(&g).contains(&"c.mp4".to_string()));
    assert!(!names(&g).contains(&"hidden.zip".to_string()));
    let g = grab(
        &client,
        &start,
        &Options {
            robots: false,
            ..options(1)
        },
        &CancelToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert!(names(&g).contains(&"hidden.zip".to_string()));

    // A type filter, and the page limit.
    let g = grab(
        &client,
        &start,
        &Options {
            types: vec!["images".into(), "pdf".into()],
            ..options(2)
        },
        &CancelToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(names(&g), vec!["a.pdf", "p.jpg"]);
    let g = grab(
        &client,
        &start,
        &Options {
            max_pages: 2,
            ..options(3)
        },
        &CancelToken::new(),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(g.pages, 2);
    assert!(g.note.unwrap().contains("stopped after 2 pages"));

    // The first page failing is an error; a stop returns what was found.
    assert!(grab(
        &client,
        &format!("http://{addr}/missing.html"),
        &options(1),
        &CancelToken::new(),
        |_| {}
    )
    .await
    .is_err());
    assert!(grab(
        &client,
        "ftp://x/y",
        &options(1),
        &CancelToken::new(),
        |_| {}
    )
    .await
    .is_err());
    let stopped = CancelToken::new();
    stopped.cancel();
    let g = grab(&client, &start, &options(1), &stopped, |_| {})
        .await
        .unwrap();
    assert_eq!(g.note.as_deref(), Some("stopped"));
}
