//! Timeout tests for HttpAgentApi (#2677).

use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, DrawerInfo, HttpAgentApi, Outcome, Report,
};

fn dummy_claim_request(wait_ms: u64) -> ClaimRequest {
    ClaimRequest {
        wait_ms,
        version: "1.0.0".to_owned(),
        build: "abc".to_owned(),
        os: "linux".to_owned(),
        host: "test-host".to_owned(),
        drawer: DrawerInfo {
            transport: "CUPS".to_owned(),
            target: "POS80".to_owned(),
        },
    }
}

fn dummy_report() -> Report {
    Report {
        job_id: "job-1".to_owned(),
        lease_id: "lease-1".to_owned(),
        outcome: Outcome::DrawerOpened,
        reason: "".to_owned(),
        detail: serde_json::json!({}),
    }
}

/// Row 1: Server answers 204 within wait_ms (existing behaviour, keep) -> Ok(None).
#[tokio::test]
async fn server_answers_204_within_wait_ms() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().expect("port").port();

    let _server = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let response = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n";
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "token").expect("create api");
    let req = dummy_claim_request(1000);

    let result = api.claim(&req).await;
    assert_eq!(result, Ok(None));
}

/// Row 2: Server answers 200 after wait_ms - 100 ms (long-poll near its end) -> Ok(Some(_)).
/// The timeout must not cut a valid long poll.
#[tokio::test]
async fn server_answers_200_after_wait_ms_minus_100ms_long_poll() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().expect("port").port();

    let wait_ms = 600u64;
    let delay_ms = wait_ms - 100;

    let _server = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            let body = r#"{"jobId":"job-99","leaseId":"lease-99","bytes":"a2lja19ieXRlcw==","sendWithinMs":3000}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });

    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "token").expect("create api");
    let req = dummy_claim_request(wait_ms);

    let result = tokio::time::timeout(Duration::from_millis(wait_ms + 2000), api.claim(&req))
        .await
        .expect("claim timed out");

    assert_eq!(
        result,
        Ok(Some(Claimed {
            job_id: "job-99".to_owned(),
            lease_id: "lease-99".to_owned(),
            bytes: "a2lja19ieXRlcw==".to_owned(),
            send_within_ms: 3000,
        }))
    );
}

/// Row 3: Server accepts the connection on claim and never answers, wait_ms = 0
/// -> Err(ApiError::Connection(_)) within MARGIN + 2 s.
#[tokio::test]
async fn server_accepts_on_claim_and_never_answers_wait_ms_0() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().expect("port").port();

    let _server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });

    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "token").expect("create api");
    let req = dummy_claim_request(0);

    let start = Instant::now();
    // Bounded at 12 s (MARGIN + 2 s).
    let result = tokio::time::timeout(Duration::from_secs(12), api.claim(&req))
        .await
        .expect("claim hung: test timeout reached");

    assert!(
        matches!(result, Err(ApiError::Connection(_))),
        "expected ApiError::Connection, got {result:?}"
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed <= Duration::from_secs(12),
        "claim timeout must complete within MARGIN + 2 s, took {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(9800),
        "claim timeout should take approximately MARGIN (10 s), took {elapsed:?}"
    );
}

/// Row 4: Server accepts the connection on report and never answers
/// -> Err(ApiError::Connection(_)) within the report timeout + 2 s.
#[tokio::test]
async fn server_accepts_on_report_and_never_answers() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().expect("port").port();

    let _server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });

    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "token").expect("create api");
    let rep = dummy_report();

    let start = Instant::now();
    // Bounded at 17 s (report timeout + 2 s).
    let result = tokio::time::timeout(Duration::from_secs(17), api.report(&rep))
        .await
        .expect("report hung: test timeout reached");

    assert!(
        matches!(result, Err(ApiError::Connection(_))),
        "expected ApiError::Connection, got {result:?}"
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed <= Duration::from_secs(17),
        "report timeout must complete within report timeout + 2 s, took {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(14800),
        "report timeout should take approximately REPORT_TIMEOUT (15 s), took {elapsed:?}"
    );
}

/// Row 5: Server answers 401 or 429 (existing behaviour, keep)
/// -> Unauthorized / RateLimited, unchanged.
#[tokio::test]
async fn server_answers_401_or_429_existing_behaviour() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().expect("port").port();

    let _server = tokio::spawn(async move {
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
    let req = dummy_claim_request(1000);

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
