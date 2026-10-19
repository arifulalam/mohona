//! Downloads through HTTP and SOCKS5 proxies, proven with proxies that
//! count what they relay.

use std::net::SocketAddr;

use mohona_net::{ClientConfig, HttpClient, NetError, ProxyChoice, RequestContext};
use mohona_testserver::proxy::{start_http_proxy, start_socks5_proxy, ProxyHandle};
use mohona_testserver::{bind, serve, Config};

async fn start() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve(
            listener,
            Config {
                default_size: 150_000,
            },
        )
        .await;
    });
    addr
}

fn through(proxy: &str) -> HttpClient {
    let client = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    let context = RequestContext {
        proxy: Some(ProxyChoice::parse(proxy).unwrap()),
        ..RequestContext::default()
    };
    client.with_context(&context).unwrap()
}

async fn fetch(client: &HttpClient, url: &str) -> Result<usize, NetError> {
    let mut body = client.get(url).await?;
    let mut total = 0;
    while let Some(chunk) = body.chunk().await? {
        total += chunk.len();
    }
    Ok(total)
}

async fn all_three_requests_work(client: &HttpClient, url: &str) {
    let probe = client.probe(url).await.unwrap();
    assert_eq!(probe.size, Some(150_000));
    assert!(probe.accepts_ranges);
    assert_eq!(fetch(client, url).await.unwrap(), 150_000);
    let mut part = client.get_range(url, 10, 109, None).await.unwrap();
    assert_eq!(part.chunk().await.unwrap().unwrap().len(), 100);
}

#[tokio::test]
async fn an_http_proxy_carries_the_download() {
    let server = start().await;
    let proxy = start_http_proxy(None).await;
    let url = format!("http://{server}/file");
    all_three_requests_work(&through(&format!("http://{}", proxy.addr)), &url).await;
    assert!(proxy.relayed() >= 3, "relayed {}", proxy.relayed());
    assert!(proxy
        .targets
        .lock()
        .unwrap()
        .iter()
        .all(|t| *t == server.to_string()));
}

#[tokio::test]
async fn an_http_proxy_with_a_login() {
    let server = start().await;
    let proxy = start_http_proxy(Some(("pat", "pw"))).await;
    let url = format!("http://{server}/file");

    // Without the login: a clear error, and nothing was relayed.
    let err = fetch(&through(&format!("http://{}", proxy.addr)), &url)
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::ProxyAuthRequired), "{err:?}");
    assert_eq!(proxy.relayed(), 0);

    // With it in the proxy address.
    all_three_requests_work(&through(&format!("http://pat:pw@{}", proxy.addr)), &url).await;
    assert!(proxy.relayed() >= 3);
}

async fn socks_works(proxy: &ProxyHandle, scheme: &str, userinfo: &str, server: SocketAddr) {
    let before = proxy.relayed();
    let url = format!("http://{server}/file");
    all_three_requests_work(
        &through(&format!("{scheme}://{userinfo}{}", proxy.addr)),
        &url,
    )
    .await;
    assert!(
        proxy.relayed() > before,
        "{scheme}: traffic must go through"
    );
}

#[tokio::test]
async fn a_socks5_proxy_carries_the_download() {
    let server = start().await;
    let proxy = start_socks5_proxy(None).await;
    socks_works(&proxy, "socks5", "", server).await;
    socks_works(&proxy, "socks5h", "", server).await;
}

#[tokio::test]
async fn a_socks5_proxy_with_a_login() {
    let server = start().await;
    let proxy = start_socks5_proxy(Some(("sam", "pw"))).await;
    socks_works(&proxy, "socks5", "sam:pw@", server).await;
    // Wrong password and no login at all: nothing goes through.
    let url = format!("http://{server}/file");
    let before = proxy.relayed();
    assert!(
        fetch(&through(&format!("socks5://sam:bad@{}", proxy.addr)), &url)
            .await
            .is_err()
    );
    assert!(fetch(&through(&format!("socks5://{}", proxy.addr)), &url)
        .await
        .is_err());
    assert_eq!(proxy.relayed(), before);
}

#[tokio::test]
async fn socks5h_sends_the_name_and_socks5_sends_an_address() {
    let server = start().await;
    let proxy = start_socks5_proxy(None).await;
    let port = server.port();
    let url = format!("http://localhost:{port}/file");
    fetch(&through(&format!("socks5h://{}", proxy.addr)), &url)
        .await
        .unwrap();
    fetch(&through(&format!("socks5://{}", proxy.addr)), &url)
        .await
        .unwrap();
    let targets = proxy.targets.lock().unwrap().clone();
    assert!(
        targets.contains(&format!("localhost:{port}")),
        "{targets:?}"
    );
    assert!(
        targets
            .iter()
            .any(|t| t.starts_with("127.0.0.1:") || t.starts_with("[::1]:")),
        "{targets:?}"
    );
}

#[tokio::test]
async fn a_download_can_choose_no_proxy_and_a_dead_proxy_is_an_error() {
    let server = start().await;
    let url = format!("http://{server}/file");
    // Direct needs no proxy at all.
    let client = HttpClient::new(&ClientConfig::default()).unwrap();
    let direct = client
        .with_context(&RequestContext {
            proxy: Some(ProxyChoice::Direct),
            ..RequestContext::default()
        })
        .unwrap();
    assert_eq!(fetch(&direct, &url).await.unwrap(), 150_000);
    // A proxy that is not there: the download cannot happen.
    assert!(fetch(&through("http://127.0.0.1:9"), &url).await.is_err());
}

#[test]
fn proxy_addresses_are_checked() {
    assert!(matches!(
        ProxyChoice::parse("direct"),
        Ok(ProxyChoice::Direct)
    ));
    assert!(matches!(ProxyChoice::parse("OFF"), Ok(ProxyChoice::Direct)));
    assert_eq!(
        ProxyChoice::parse("host:3128").unwrap(),
        ProxyChoice::Url("http://host:3128".into())
    );
    assert!(ProxyChoice::parse("socks5h://127.0.0.1:1080").is_ok());
    assert!(ProxyChoice::parse("ftp://host").is_err());
    assert!(ProxyChoice::parse("http://").is_err());
    let hidden = format!("{:?}", ProxyChoice::parse("http://u:secret@h:1").unwrap());
    assert!(!hidden.contains("secret"));
}
