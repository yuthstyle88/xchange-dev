//! Specification tests for ticket #52: the settings a deploy tunes reach the
//! pool and the server.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `database.min_connections` and `database.max_connections` reach the pool
//!   `bootstrap::connect` opens. Both are set to values neither sqlx's defaults
//!   (0 and 10) nor `base.yaml` (1 and 10) use, so a dropped call shows.
//! - `application.shutdown_timeout_secs` reaches the server `Application::build`
//!   starts. actix has no getter for it, so the test watches it: a request is
//!   held open (a body announced and never sent), the process gets `SIGTERM`
//!   (a graceful stop, the signal a deploy sends), and the server must be gone
//!   after the configured 2 s — not after actix's default 30 s, and not at once
//!   (which would mean the held request was not waited for at all).
//!
//! The tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name
//! ends in `_test`, a role that may create databases). Without it they FAIL;
//! none skips.
//!
//! The `SIGTERM` reaches the whole test process. Once the server has started,
//! actix listens for it, so the process lives on; no other test in this file
//! runs a server that it could stop.

mod common;
mod routes_common;

use std::io::Write;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use routes_common::{ScratchDatabase, http_get};
use xchange::bootstrap::{self, Application};

/// The shutdown timeout the server test configures.
const SHUTDOWN_TIMEOUT_SECS: u64 = 2;
/// The server must be gone no sooner than this after `SIGTERM` ...
const GONE_NOT_BEFORE: Duration = Duration::from_millis(1500);
/// ... and no later than this (actix's default would take 30 s).
const GONE_BY: Duration = Duration::from_millis(6000);
/// How long the test waits for the server at most before it fails.
const GIVE_UP_AFTER: Duration = Duration::from_secs(12);

#[actix_web::test]
async fn the_pool_keeps_the_minimum_the_settings_ask_for() {
    let db = ScratchDatabase::create("t1b_min_conns").await;
    let mut settings = db.settings();
    settings.database.min_connections = 3;
    settings.database.max_connections = 7;

    let pool = bootstrap::connect(&settings)
        .await
        .expect("connect to the scratch database");

    assert_eq!(pool.options().get_min_connections(), 3);

    pool.close().await;
    db.drop().await;
}

#[actix_web::test]
async fn the_pool_keeps_the_maximum_the_settings_ask_for() {
    let db = ScratchDatabase::create("t1b_max_conns").await;
    let mut settings = db.settings();
    settings.database.min_connections = 1;
    settings.database.max_connections = 7;

    let pool = bootstrap::connect(&settings)
        .await
        .expect("connect to the scratch database");

    assert_eq!(pool.options().get_max_connections(), 7);

    pool.close().await;
    db.drop().await;
}

#[cfg(unix)]
#[actix_web::test]
async fn a_graceful_stop_waits_as_long_as_the_settings_say() {
    let db = ScratchDatabase::create("t1b_shutdown").await;
    let mut settings = db.settings();
    settings.application.shutdown_timeout_secs = SHUTDOWN_TIMEOUT_SECS;

    let application = Application::build(settings)
        .await
        .expect("start on an empty, reachable database");
    let port = application.port();
    let server = actix_web::rt::spawn(application.run_until_stopped());

    // One answered request: the server runs, and so does its signal listener
    // (actix sets both up in the same step, before any worker answers).
    let (health, _) = tokio::task::spawn_blocking(move || http_get(port, "/healthz"))
        .await
        .expect("the client thread");
    assert!(health.contains(" 200 "), "{health}");

    // Use HTTP/1.1 keep-alive on the held connection for readiness without sleeping (PR 204):
    // 1. Sends GET /healthz and reads its complete 200 answer. Only a worker writes
    //    that answer, so the connection belongs to a worker.
    // 2. On the same connection, sends the head of POST /api/staff/login with
    //    Content-Length: 64 and no body. The same worker now waits for the body.
    let held = tokio::task::spawn_blocking(move || {
        use std::io::Read;

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
            "POST /api/staff/login HTTP/1.1\r\nHost: 127.0.0.1\r\n\
             Content-Type: application/json\r\nContent-Length: 64\r\n\r\n"
        )
        .expect("send POST /api/staff/login head");
        stream.flush().expect("flush POST head");

        stream
    })
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
