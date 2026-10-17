//! Ticket #2205 — `HttpAgentApi::report` maps 5xx responses to `ApiError::Connection(_)` so the
//! runner keeps the report owed instead of marking it rejected for good.

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use xc_device_agent::client::{AgentApi, ApiError, HttpAgentApi, Outcome, Report};

fn sample_report() -> Report {
    Report {
        job_id: "job-2205".to_owned(),
        lease_id: "lease-2205".to_owned(),
        outcome: Outcome::DrawerOpened,
        reason: String::new(),
        detail: json!({"cupsJob": "POS80-1"}),
    }
}

async fn serve_once(
    status_line: &'static str,
    extra_headers: &'static str,
    body: &'static str,
) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().expect("port").port();
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 {status_line}\r\n{extra_headers}Content-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
        }
    });
    port
}

/// Row 1: 503 -> `Err(ApiError::Connection(_))`.
#[tokio::test]
async fn report_503_returns_connection_error_so_report_stays_owed() {
    let port = serve_once("503 Service Unavailable", "", "upstream unavailable").await;
    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "tok-2205").expect("api");

    let got = api.report(&sample_report()).await;
    match got {
        Err(ApiError::Connection(msg)) => {
            assert!(
                msg.contains("503"),
                "expected Connection error naming status 503, got: {msg}"
            );
        }
        other => panic!("expected Err(ApiError::Connection(_)) on 503, got {other:?}"),
    }
}

/// Row 2: 500, 502, 504 -> `Err(ApiError::Connection(_))`.
#[tokio::test]
async fn report_500_502_504_return_connection_error() {
    for (status_line, code) in [
        ("500 Internal Server Error", "500"),
        ("502 Bad Gateway", "502"),
        ("504 Gateway Timeout", "504"),
    ] {
        let port = serve_once(status_line, "", "server failure").await;
        let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "tok-2205").expect("api");
        let got = api.report(&sample_report()).await;
        match got {
            Err(ApiError::Connection(msg)) => {
                assert!(
                    msg.contains(code),
                    "expected Connection error naming status {code}, got: {msg}"
                );
            }
            other => panic!("expected Err(ApiError::Connection(_)) on {code}, got {other:?}"),
        }
    }
}

/// Row 3: 409 -> `Err(ApiError::Rejected { status: 409, .. })`.
#[tokio::test]
async fn report_409_returns_rejected_and_5xx_returns_connection() {
    let port_409 = serve_once("409 Conflict", "", "already recorded").await;
    let api_409 =
        HttpAgentApi::new(format!("http://127.0.0.1:{port_409}"), "tok-2205").expect("api");
    assert_eq!(
        api_409.report(&sample_report()).await,
        Err(ApiError::Rejected {
            status: 409,
            error: "already recorded".to_owned(),
        })
    );
}

/// Row 4: 404 -> `Err(ApiError::Rejected { status: 404, .. })`.
#[tokio::test]
async fn report_404_returns_rejected() {
    let port_404 = serve_once("404 Not Found", "", "unknown job").await;
    let api_404 =
        HttpAgentApi::new(format!("http://127.0.0.1:{port_404}"), "tok-2205").expect("api");
    assert_eq!(
        api_404.report(&sample_report()).await,
        Err(ApiError::Rejected {
            status: 404,
            error: "unknown job".to_owned(),
        })
    );
}

/// Row 5: 401 -> `Err(ApiError::Unauthorized)`.
#[tokio::test]
async fn report_401_returns_unauthorized() {
    let port = serve_once("401 Unauthorized", "", "").await;
    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "tok-2205").expect("api");
    assert_eq!(
        api.report(&sample_report()).await,
        Err(ApiError::Unauthorized)
    );
}

/// Row 6: 429 + `Retry-After: 7` -> `Err(ApiError::RateLimited { retry_after_secs: 7 })`.
#[tokio::test]
async fn report_429_returns_rate_limited_with_retry_after() {
    let port = serve_once("429 Too Many Requests", "Retry-After: 7\r\n", "").await;
    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "tok-2205").expect("api");
    assert_eq!(
        api.report(&sample_report()).await,
        Err(ApiError::RateLimited {
            retry_after_secs: 7
        })
    );
}

/// Row 7: 204 -> `Ok(())`.
#[tokio::test]
async fn report_204_returns_ok() {
    let port = serve_once("204 No Content", "", "").await;
    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), "tok-2205").expect("api");
    assert_eq!(api.report(&sample_report()).await, Ok(()));
}

/// Row 8: 5xx body containing the bearer token -> `Connection` text has no token (redacted).
#[tokio::test]
async fn report_5xx_body_with_bearer_token_is_redacted_in_connection_error() {
    let secret = "xcda_super_secret_bearer_2205";
    let port = serve_once(
        "502 Bad Gateway",
        "",
        "upstream failed for token xcda_super_secret_bearer_2205",
    )
    .await;
    let api = HttpAgentApi::new(format!("http://127.0.0.1:{port}"), secret).expect("api");

    let got = api.report(&sample_report()).await;
    match got {
        Err(ApiError::Connection(msg)) => {
            assert!(
                !msg.contains(secret),
                "bearer token leaked in Connection error: {msg}"
            );
            assert!(
                msg.contains("502"),
                "expected status 502 in Connection error: {msg}"
            );
        }
        other => panic!("expected Err(ApiError::Connection(_)) on 502, got {other:?}"),
    }
}
