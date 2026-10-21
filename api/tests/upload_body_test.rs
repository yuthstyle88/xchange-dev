//! Written before the implementation (ticket #124, part (a)). Do not edit, delete or weaken.
//!
//! `POST /api/upload` and `POST /api/staff/upload` must know who is calling, and whether the
//! origin is accepted, **before** they read the body. A refusal that does not depend on the body
//! comes without the body being read.
//!
//! How "not read" is observed: a real server on a free port, and a client that sends only the
//! request head with `Content-Length` of 15 MB + 1 and then waits. A handler that reads the body
//! first waits for bytes that never come, so no answer arrives; a handler that decides from the
//! headers answers at once. The answer is compared with the one the same request gets with an
//! empty body, so nothing here says *which* refusal a route gives — only that it is the same one,
//! and early.
//!
//! The refusals need no database (no session, a refused origin, a refused role, all decided from
//! headers). The served caller's case, last, needs `TEST_DATABASE_URL` like every file route test.

mod common;
#[macro_use]
mod file_routes_common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use actix_web::{App, HttpServer};
use file_routes_common::{AS, Desk, FakeSessions, HOST, JPEG, ORIGIN, fixture};
use serde_json::Value;
use xchange::bootstrap::{AppState, configure};
use xchange::presentation::pages::SessionReader;

/// One byte over `MAX_UPLOAD_BYTES` (`file_service.rs:12`).
const OVER_THE_LIMIT: usize = 15 * 1024 * 1024 + 1;
/// How long an answer that needs no body may take.
const AT_ONCE: Duration = Duration::from_secs(5);

fn start(state: AppState, fixture: &Value) -> SocketAddr {
    let fixture = fixture.clone();
    let server = HttpServer::new(move || {
        let reader: Arc<dyn SessionReader> = Arc::new(FakeSessions::from_fixture(&fixture));
        let state = state.clone();
        App::new()
            .configure(move |cfg| configure(cfg, state))
            .app_data(actix_web::web::Data::from(reader))
    })
    .workers(1)
    .bind(("127.0.0.1", 0))
    .expect("bind a free port");
    let addr = server.addrs()[0];
    actix_web::rt::spawn(server.run());
    addr
}

/// The request head of an upload, with the `Content-Length` given.
fn head(path: &str, as_who: &str, origin: &str, length: usize) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: {HOST}\r\nX-Forwarded-Proto: https\r\nOrigin: {origin}\r\n\
         X-Xchange-Request: 1\r\nContent-Type: image/jpeg\r\n{AS}: {as_who}\r\n\
         Content-Length: {length}\r\nConnection: close\r\n\r\n"
    )
}

/// The status line and body of the answer, or `None` when none came within `wait`.
fn exchange(
    addr: SocketAddr,
    head: String,
    body: Vec<u8>,
    wait: Duration,
) -> Option<(String, String)> {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(wait)).unwrap();
    stream.write_all(head.as_bytes()).expect("send the head");
    // the server may answer and close while the body is still going out: that is its right
    let _ = stream.write_all(&body);
    let mut got = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if let Some(end) = find(&got, b"\r\n\r\n") {
                    let text = String::from_utf8_lossy(&got[..end]).to_ascii_lowercase();
                    let length = text
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok());
                    if length.is_some_and(|len| got.len() >= end + 4 + len) {
                        break;
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    let end = find(&got, b"\r\n\r\n")?;
    let status = String::from_utf8_lossy(&got[..end])
        .lines()
        .next()
        .unwrap_or("")
        .to_owned();
    let body = String::from_utf8_lossy(&got[end + 4..]).into_owned();
    Some((status, body))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The answer with an empty body, then the same request announcing 15 MB + 1 and sending
/// nothing: it must be the same answer, and it must come at once.
async fn refused_without_reading(addr: SocketAddr, path: &str, as_who: &str, origin: &str) {
    let (path, as_who, origin) = (path.to_owned(), as_who.to_owned(), origin.to_owned());
    actix_web::rt::task::spawn_blocking(move || {
        let what = format!("{path} as {as_who:?}, Origin {origin}");
        let empty = exchange(addr, head(&path, &as_who, &origin, 0), Vec::new(), AT_ONCE)
            .unwrap_or_else(|| panic!("{what}: no answer to an empty body"));
        assert!(
            !empty.0.contains(" 200 "),
            "{what}: this case must be a refusal, got {}",
            empty.0
        );
        let announced = exchange(
            addr,
            head(&path, &as_who, &origin, OVER_THE_LIMIT),
            Vec::new(),
            AT_ONCE,
        )
        .unwrap_or_else(|| {
            panic!("{what}: no answer within {AT_ONCE:?} — the handler is waiting for the body")
        });
        assert_eq!(
            announced, empty,
            "{what}: the same answer, without the body"
        );
    })
    .await
    .expect("the client thread");
}

fn offline_state() -> AppState {
    AppState::new(Arc::new(common::settings(&[])), common::unreachable_pool())
}

#[actix_web::test]
async fn nobody_signed_in_is_refused_before_the_body_is_read() {
    let addr = start(offline_state(), &fixture());
    refused_without_reading(addr, "/api/upload", "", ORIGIN).await;
    refused_without_reading(addr, "/api/staff/upload", "", ORIGIN).await;
}

#[actix_web::test]
async fn a_refused_origin_is_refused_before_the_body_is_read() {
    let addr = start(offline_state(), &fixture());
    let elsewhere = "https://elsewhere.example";
    refused_without_reading(addr, "/api/upload", "owner", elsewhere).await;
    refused_without_reading(addr, "/api/upload", "front", elsewhere).await;
    refused_without_reading(addr, "/api/staff/upload", "front", elsewhere).await;
}

#[actix_web::test]
async fn a_refused_role_is_refused_before_the_body_is_read() {
    let addr = start(offline_state(), &fixture());
    for who in ["rates", "system", "monitor", "manager"] {
        refused_without_reading(addr, "/api/upload", who, ORIGIN).await;
        refused_without_reading(addr, "/api/staff/upload", who, ORIGIN).await;
    }
    // the owner on the staff route
    refused_without_reading(addr, "/api/staff/upload", "owner", ORIGIN).await;
}

#[actix_web::test]
async fn a_served_caller_s_body_is_still_read_and_refused_by_the_service() {
    let fixture = fixture();
    let desk = Desk::start().await;
    let addr = start(desk.state(), &fixture);
    actix_web::rt::task::spawn_blocking(move || {
        // a JPEG that is one byte over the limit
        let mut body = JPEG.to_vec();
        body.resize(OVER_THE_LIMIT, 0);
        for (path, who) in [("/api/upload", "owner"), ("/api/staff/upload", "front")] {
            let answer = exchange(
                addr,
                head(path, who, ORIGIN, OVER_THE_LIMIT),
                body.clone(),
                Duration::from_secs(60),
            )
            .unwrap_or_else(|| panic!("{path} as {who}: no answer"));
            assert!(answer.0.contains(" 400 "), "{path} as {who}: {answer:?}");
            assert!(
                answer.1.contains("ไฟล์ใหญ่เกิน 15 MB"),
                "{path} as {who}: the service's own sentence: {answer:?}"
            );
            // and a served caller does wait for the body it announced
            assert!(
                exchange(
                    addr,
                    head(path, who, ORIGIN, OVER_THE_LIMIT),
                    Vec::new(),
                    Duration::from_secs(2)
                )
                .is_none(),
                "{path} as {who}: answered without reading the body of a caller it serves"
            );
        }
    })
    .await
    .expect("the client thread");
    assert!(desk.rows().await.is_empty(), "nothing was stored");
    desk.finish().await;
}
