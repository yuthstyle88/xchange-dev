//! Specification test for ticket #198: the configured shutdown timeout (short, 1 s)
//! is honoured by the server on a graceful stop.
//!
//! Follow-up of PR 185. Proves that application.shutdown_timeout_secs is wired to
//! actix's .shutdown_timeout(..) and distinguishes it from constants and defaults.

mod common;
mod routes_common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use routes_common::ScratchDatabase;
use xchange::bootstrap::Application;

/// The shutdown timeout this test configures.
const SHUTDOWN_TIMEOUT_SECS: u64 = 1;
/// The server must be gone no sooner than 0.8 s after SIGTERM ...
const GONE_NOT_BEFORE: Duration = Duration::from_millis(800);
/// ... and no later than 1.8 s.
const GONE_BY: Duration = Duration::from_millis(1800);
/// How long the test waits for the server at most before it fails.
const GIVE_UP_AFTER: Duration = Duration::from_secs(12);

/// Use HTTP/1.1 keep-alive on the held connection for readiness without sleeping.
///
/// 1. Sends GET /healthz and reads its complete 200 answer. Only a worker writes
///    that answer, so the connection belongs to a worker.
/// 2. On the same connection, sends the head of POST /api/auth/login with
///    Content-Length: 64 and no body. The same worker now waits for the body.
fn prepare_held_connection(port: u16) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to the server");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set a read timeout");

    write!(
        stream,
        "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: keep-alive\r\n\r\n"
    )
    .expect("send GET /healthz");
    stream.flush().expect("flush GET /healthz");

    let mut header_bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !header_bytes.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .expect("read /healthz header byte; keep-alive not held");
        header_bytes.push(byte[0]);
    }

    let header_str = String::from_utf8_lossy(&header_bytes);
    assert!(
        header_str.contains(" 200 "),
        "healthz did not return 200: {header_str}"
    );

    let content_length: usize = header_str
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-length:") {
                lower
                    .split_once(':')
                    .and_then(|(_, v)| v.trim().parse().ok())
            } else {
                None
            }
        })
        .unwrap_or(0);

    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        stream.read_exact(&mut body).expect("read /healthz body");
    }

    write!(
        stream,
        "POST /api/auth/login HTTP/1.1\r\nHost: 127.0.0.1\r\n\
         Content-Type: application/json\r\nContent-Length: 64\r\n\r\n"
    )
    .expect("send POST /api/auth/login head");
    stream.flush().expect("flush POST head");

    stream
}

#[cfg(unix)]
#[actix_web::test]
async fn a_graceful_stop_waits_for_the_short_timeout() {
    let db = ScratchDatabase::create("t1b_shutdown_short").await;
    let mut settings = db.settings();
    settings.application.shutdown_timeout_secs = SHUTDOWN_TIMEOUT_SECS;

    common::install_test_pii_keys();
    let application = Application::build(settings)
        .await
        .expect("start on an empty, reachable database");
    let port = application.port();
    let server = actix_web::rt::spawn(application.run_until_stopped());

    let held = tokio::task::spawn_blocking(move || prepare_held_connection(port))
        .await
        .expect("the client thread");

    let stopped_at = Instant::now();
    let kill = std::process::Command::new("kill")
        .args(["-TERM", &std::process::id().to_string()])
        .status()
        .expect("run kill");
    assert!(kill.success(), "kill -TERM failed: {kill}");

    let outcome = tokio::time::timeout(GIVE_UP_AFTER, server).await;
    let took = stopped_at.elapsed();
    drop(held);
    println!("shutdown_timeout_short_test: took {took:?}");

    let ended = outcome.unwrap_or_else(|_| {
        panic!(
            "the server was still running {GIVE_UP_AFTER:?} after SIGTERM; \
             application.shutdown_timeout_secs = {SHUTDOWN_TIMEOUT_SECS} was not applied"
        )
    });
    ended
        .expect("the server task")
        .expect("the server ends without an error");
    assert!(
        took >= GONE_NOT_BEFORE,
        "the server was gone {took:?} after SIGTERM: the held request was not waited for, \
         so this test does not show the timeout"
    );
    assert!(
        took <= GONE_BY,
        "the server was gone {took:?} after SIGTERM; with \
         shutdown_timeout_secs = {SHUTDOWN_TIMEOUT_SECS} it must be gone by {GONE_BY:?}"
    );

    db.drop().await;
}
