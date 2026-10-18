//! Cookies, referrer, user agent and headers reach the server.

use std::net::SocketAddr;

use mohona_net::{ClientConfig, HttpClient, NetError, RequestContext};
use mohona_testserver::{bind, serve, Config};

async fn start() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve(
            listener,
            Config {
                default_size: 100_000,
            },
        )
        .await;
    });
    addr
}

fn client() -> HttpClient {
    HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap()
}

fn context() -> RequestContext {
    let mut c = RequestContext {
        cookies: Some("theme=dark; session=ok".into()),
        referrer: Some("https://example.org/page".into()),
        ..RequestContext::default()
    };
    c.add_header_line("X-Token: abc").unwrap();
    c
}

async fn read_all(mut body: mohona_net::Body) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(chunk) = body.chunk().await.unwrap() {
        out.extend_from_slice(&chunk);
    }
    out
}

#[tokio::test]
async fn the_guarded_page_needs_all_three() {
    let addr = start().await;
    let url = format!("http://{addr}/guard");

    // Nothing: refused.
    let err = client().get(&url).await.err().expect("must be refused");
    assert!(matches!(err, NetError::Status { status: 403, .. }));

    // Only part of it: still refused.
    let partial = RequestContext {
        cookies: Some("session=ok".into()),
        ..RequestContext::default()
    };
    let err = client()
        .with_context(&partial)
        .unwrap()
        .get(&url)
        .await
        .err()
        .expect("must be refused");
    assert!(matches!(err, NetError::Status { status: 403, .. }));

    // Everything: served, for probe, plain GET and ranged GET.
    let full = client().with_context(&context()).unwrap();
    let probe = full.probe(&url).await.unwrap();
    assert_eq!(probe.size, Some(100_000));
    assert!(probe.accepts_ranges);
    assert_eq!(read_all(full.get(&url).await.unwrap()).await.len(), 100_000);
    let part = full.get_range(&url, 10, 19, None).await.unwrap();
    assert_eq!(read_all(part).await.len(), 10);
}

#[tokio::test]
async fn the_user_agent_can_be_replaced_and_other_headers_arrive() {
    let addr = start().await;
    let url = format!("http://{addr}/echo");
    let mut c = RequestContext {
        user_agent: Some("Browser/9".into()),
        ..RequestContext::default()
    };
    c.add_header_line("Accept-Language: bn").unwrap();
    let text = String::from_utf8(
        read_all(client().with_context(&c).unwrap().get(&url).await.unwrap()).await,
    )
    .unwrap();
    assert!(text.contains("user-agent: Browser/9\n"), "{text}");
    assert!(text.contains("accept-language: bn\n"), "{text}");
    assert!(text.contains("accept-encoding: identity\n"), "{text}");

    // Without a context the default identity is sent.
    let text = String::from_utf8(read_all(client().get(&url).await.unwrap()).await).unwrap();
    assert!(text.contains("user-agent: Mohona/"), "{text}");
}

#[tokio::test]
async fn a_bad_context_is_refused_up_front() {
    let c = RequestContext {
        cookies: Some("a=1\r\nX-Evil: 1".into()),
        ..RequestContext::default()
    };
    assert!(matches!(
        client().with_context(&c),
        Err(NetError::InvalidHeader(_))
    ));
}
