//! Basic and Digest authentication against the local test server.

use std::net::SocketAddr;

use mohona_net::{ClientConfig, Credentials, HttpClient, NetError, RequestContext};
use mohona_testserver::{bind, serve, Config};

async fn start() -> SocketAddr {
    let listener = bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = serve(
            listener,
            Config {
                default_size: 200_000,
            },
        )
        .await;
    });
    addr
}

fn client_with(user: &str, password: &str) -> HttpClient {
    let client = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    let context = RequestContext {
        credentials: Some(Credentials {
            username: user.into(),
            password: password.into(),
        }),
        ..RequestContext::default()
    };
    client.with_context(&context).unwrap()
}

fn good() -> HttpClient {
    client_with("mohona", "secret")
}

async fn size_of(client: &HttpClient, url: &str) -> usize {
    let mut body = client.get(url).await.unwrap();
    let mut total = 0;
    while let Some(chunk) = body.chunk().await.unwrap() {
        total += chunk.len();
    }
    total
}

fn is_401(error: &NetError) -> bool {
    matches!(error, NetError::Status { status: 401, .. })
}

#[tokio::test]
async fn basic_works_only_with_the_right_password() {
    let addr = start().await;
    let url = format!("http://{addr}/auth");
    let client = good();
    let probe = client.probe(&url).await.unwrap();
    assert_eq!(probe.size, Some(200_000));
    assert!(probe.accepts_ranges);
    assert_eq!(size_of(&client, &url).await, 200_000);
    let mut part = client.get_range(&url, 100, 199, None).await.unwrap();
    assert_eq!(part.chunk().await.unwrap().unwrap().len(), 100);

    assert!(is_401(
        &client_with("mohona", "wrong")
            .get(&url)
            .await
            .err()
            .unwrap()
    ));
    let none = HttpClient::new(&ClientConfig {
        use_system_proxy: false,
        ..ClientConfig::default()
    })
    .unwrap();
    assert!(is_401(&none.get(&url).await.err().unwrap()));
}

#[tokio::test]
async fn digest_works_with_md5_sha256_and_the_old_form() {
    let addr = start().await;
    for query in ["", "?alg=sha256", "?qop=none", "?alg=sha256&qop=none"] {
        let url = format!("http://{addr}/digest{query}");
        let client = good();
        let probe = client.probe(&url).await.unwrap();
        assert_eq!(probe.size, Some(200_000), "{query}");
        assert_eq!(size_of(&client, &url).await, 200_000, "{query}");
        let mut part = client.get_range(&url, 0, 9, None).await.unwrap();
        assert_eq!(part.chunk().await.unwrap().unwrap().len(), 10, "{query}");
        assert!(
            is_401(&client_with("mohona", "nope").get(&url).await.err().unwrap()),
            "{query}"
        );
    }
}

#[tokio::test]
async fn many_connections_at_once_all_get_through() {
    let addr = start().await;
    // The query makes each address different but the server serves the same
    // file. The nonce is shared; the order its counter reaches the server in
    // is not, so some answers are refused and must be redone.
    let url = format!("http://{addr}/digest?size=100000");
    let client = good();
    let mut tasks = Vec::new();
    for i in 0..16u64 {
        let client = client.clone();
        let url = url.clone();
        tasks.push(tokio::spawn(async move {
            let mut body = client
                .get_range(&url, i * 100, i * 100 + 99, None)
                .await
                .unwrap();
            let mut n = 0;
            while let Some(chunk) = body.chunk().await.unwrap() {
                n += chunk.len();
            }
            n
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), 100);
    }
}

#[tokio::test]
async fn an_expired_nonce_is_renewed_without_failing() {
    let addr = start().await;
    let url = format!("http://{addr}/digest?expire=2&size=50000");
    let client = good();
    for _ in 0..7 {
        assert_eq!(size_of(&client, &url).await, 50_000);
    }
}

#[tokio::test]
async fn credentials_are_not_given_to_another_server() {
    let first = start().await;
    let second = start().await;
    // The first server sends the client on to the second, which wants a login.
    let url = format!("http://{first}/redirect?to=http://{second}/auth");
    let err = good().get(&url).await.err().expect("must not log in");
    assert!(is_401(&err), "{err:?}");
    // The same address works when the login is asked for by the server the
    // request was addressed to.
    assert_eq!(
        size_of(&good(), &format!("http://{second}/auth")).await,
        200_000
    );
}
