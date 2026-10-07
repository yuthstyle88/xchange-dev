//! Tests for HttpAgentApi.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use xc_device_agent::client::{AgentApi, ApiError, ClaimRequest, DrawerInfo, HttpAgentApi};

#[test]
fn http_is_refused_for_non_local_hosts() {
    assert!(HttpAgentApi::new("http://counter.example.com", "token").is_err());
    assert!(HttpAgentApi::new("http://192.168.1.100", "token").is_err());
    assert!(HttpAgentApi::new("http://shop.lan:8080", "token").is_err());

    assert!(HttpAgentApi::new("https://counter.example.com", "token").is_ok());
    assert!(HttpAgentApi::new("http://localhost:8080", "token").is_ok());
    assert!(HttpAgentApi::new("http://127.0.0.1:8080", "token").is_ok());
    assert!(HttpAgentApi::new("http://[::1]:8080", "token").is_ok());
}

#[test]
fn token_never_appears_in_debug_representation() {
    let api = HttpAgentApi::new("http://localhost:8080", "super_secret_token_12345").unwrap();
    let debug_str = format!("{api:?}");
    assert!(!debug_str.contains("super_secret_token_12345"));
    assert!(debug_str.contains("<redacted>"));
}

#[tokio::test]
async fn requests_send_bearer_token_and_never_origin_or_cookie() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().expect("port").port();

    let server_task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 4096];
        let n = socket.read(&mut buf).await.expect("read");
        let raw_req = String::from_utf8_lossy(&buf[..n]).to_string();

        let response = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n";
        socket.write_all(response.as_bytes()).await.expect("write");
        raw_req
    });

    let api = HttpAgentApi::new(
        format!("http://127.0.0.1:{port}"),
        "xcda_test_token_secret_999",
    )
    .expect("create api");

    let req = ClaimRequest {
        wait_ms: 1000,
        version: "1.0.0".to_owned(),
        build: "abc".to_owned(),
        os: "linux".to_owned(),
        host: "test-host".to_owned(),
        drawer: DrawerInfo {
            transport: "CUPS".to_owned(),
            target: "POS80".to_owned(),
        },
    };

    let result = api.claim(&req).await;
    assert_eq!(result, Ok(None));

    let raw_req = server_task.await.expect("server task");
    let lower = raw_req.to_lowercase();

    // Verify Bearer token is present
    assert!(lower.contains("authorization: bearer xcda_test_token_secret_999"));

    // Verify neither Origin nor Cookie is sent
    assert!(!lower.contains("\norigin:"));
    assert!(!lower.contains("\ncookie:"));
}

#[tokio::test]
async fn unauthorized_and_rate_limited_responses_are_handled() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().expect("port").port();

    tokio::spawn(async move {
        // First connection: 401
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n";
            let _ = socket.write_all(response.as_bytes()).await;
        }

        // Second connection: 429 with Retry-After: 42
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let response =
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 42\r\nContent-Length: 0\r\n\r\n";
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "token").expect("create api");
    let req = ClaimRequest {
        wait_ms: 1000,
        version: "1.0.0".to_owned(),
        build: "abc".to_owned(),
        os: "linux".to_owned(),
        host: "test-host".to_owned(),
        drawer: DrawerInfo {
            transport: "CUPS".to_owned(),
            target: "POS80".to_owned(),
        },
    };

    let res1 = api.claim(&req).await;
    assert_eq!(res1, Err(ApiError::Unauthorized));

    let res2 = api.claim(&req).await;
    assert_eq!(
        res2,
        Err(ApiError::RateLimited {
            retry_after_secs: 42
        })
    );
}
