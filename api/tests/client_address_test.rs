//! Ticket #130 — the caller's address comes from one header, at every door.
//!
//! The server sits behind a proxy, and the proxy is the only party that knows who is calling.
//! It says so in `X-Real-IP`. A client can write `CF-Connecting-IP` and `X-Forwarded-For`
//! itself, so the server never reads them: a limit that keyed on either could be walked around
//! by changing the header on every request.
//!
//! The door tests prove which header a door keys on through the limit on sign-in attempts
//! (30 per address per quarter hour, the 31st is refused): callers the server takes for one
//! address are locked out together, callers it takes for different addresses are not.
//! Every attempt uses a username of its own, so the limit per name (10) never answers instead.
//!
//! The door tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`); without it they fail, none skips. The tests of the function itself need nothing.

mod auth_http_common;
mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use actix_web::http::header::HeaderValue;
use actix_web::test::TestRequest;
use auth_http_common::*;
use serde_json::json;
use xchange::presentation::http::client_address::{
    CLIENT_ADDRESS_HEADER, UNKNOWN_ADDRESS, client_address,
};

/// v2's sentence for the 31st attempt from one address.
const LIMIT: &str = "ลองเข้าสู่ระบบเกินกำหนด กรุณารอ 15 นาที";
/// How many attempts one address has in a window.
const ALLOWED: usize = 30;

/// The door for the owner and employees, and the door for employees only.
const SHARED_DOOR: &str = "/api/auth";
const STAFF_DOOR: &str = "/api/staff/login";

const CF: &str = "cf-connecting-ip";
const XFF: &str = "x-forwarded-for";
const REAL: &str = "x-real-ip";

/// One wrong sign-in under a username nobody else in the test uses; gives back the refusal.
async fn attempt(app: &TestApp, door: &str, who: &str, headers: &[(&str, &str)]) -> String {
    let mut call = Call::post(
        door,
        json!({ "action": "login",
                "payload": { "username": format!("nobody-{who}"), "password": "not-the-password" } }),
    );
    for (name, value) in headers {
        call = call.header(name, value);
    }
    let answer = send(app, call).await;
    assert_eq!(answer.status, 400, "{door} {who}: {}", answer.text);
    answer.error()
}

/// The owner with the right password, at the shared door.
async fn owner_attempt(app: &TestApp, headers: &[(&str, &str)]) -> Answer {
    let mut call = Call::post(
        SHARED_DOOR,
        json!({ "action": "login", "payload": { "username": "boss", "password": OWNER_PASSWORD } }),
    );
    for (name, value) in headers {
        call = call.header(name, value);
    }
    send(app, call).await
}

async fn desk() -> Desk {
    stay_inside_one_attempt_window(Duration::from_secs(180)).await;
    Desk::start().await
}

// ── case 1: the staff door ───────────────────────────────────────────────

#[actix_web::test]
async fn the_staff_door_counts_under_x_real_ip_whatever_cf_connecting_ip_says() {
    let desk = desk().await;
    let app = desk_app!(desk);
    for n in 0..ALLOWED {
        let said = attempt(
            &app,
            STAFF_DOOR,
            &format!("a{n}"),
            &[(CF, &format!("198.51.100.{n}")), (REAL, "203.0.113.7")],
        )
        .await;
        assert_eq!(said, WRONG, "attempt {} from 203.0.113.7", n + 1);
    }
    let said = attempt(
        &app,
        STAFF_DOOR,
        "a-last",
        &[(CF, "198.51.100.250"), (REAL, "203.0.113.7")],
    )
    .await;
    assert_eq!(
        said, LIMIT,
        "the 31st from one X-Real-IP is refused: a new CF-Connecting-IP on every request does \
         not walk around the limit"
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_staff_door_does_not_join_callers_that_send_one_cf_connecting_ip() {
    let desk = desk().await;
    let app = desk_app!(desk);
    for n in 0..=ALLOWED {
        let said = attempt(
            &app,
            STAFF_DOOR,
            &format!("b{n}"),
            &[(CF, "198.51.100.9"), (REAL, &format!("203.0.113.{n}"))],
        )
        .await;
        assert_eq!(
            said,
            WRONG,
            "attempt {}: each X-Real-IP is a caller of its own, whatever CF-Connecting-IP says",
            n + 1
        );
    }
    drop(app);
    desk.drop_schema().await;
}

// ── case 2: the shared door ──────────────────────────────────────────────

#[actix_web::test]
async fn the_shared_door_counts_under_x_real_ip_whatever_cf_connecting_ip_says() {
    let desk = desk().await;
    let app = desk_app!(desk);
    for n in 0..ALLOWED {
        let said = attempt(
            &app,
            SHARED_DOOR,
            &format!("c{n}"),
            &[(CF, &format!("198.51.100.{n}")), (REAL, "203.0.113.8")],
        )
        .await;
        assert_eq!(said, WRONG, "attempt {} from 203.0.113.8", n + 1);
    }
    // The 31st is the owner with the right password: the owner's branch of the door keys on
    // the same header as the employees' branch.
    let answer = owner_attempt(&app, &[(CF, "198.51.100.251"), (REAL, "203.0.113.8")]).await;
    assert_eq!(answer.status, 400, "{}", answer.text);
    assert_eq!(
        answer.error(),
        LIMIT,
        "the 31st from one X-Real-IP is refused, also for the owner"
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_shared_door_does_not_join_callers_that_send_one_cf_connecting_ip() {
    let desk = desk().await;
    let app = desk_app!(desk);
    for n in 0..=ALLOWED {
        let said = attempt(
            &app,
            SHARED_DOOR,
            &format!("d{n}"),
            &[(CF, "198.51.100.10"), (REAL, &format!("203.0.113.{n}"))],
        )
        .await;
        assert_eq!(said, WRONG, "attempt {}", n + 1);
    }
    let answer = owner_attempt(&app, &[(CF, "198.51.100.10"), (REAL, "203.0.113.200")]).await;
    assert_eq!(
        answer.status, 200,
        "the owner, from an X-Real-IP of their own, is not locked out by 31 others that sent \
         the same CF-Connecting-IP: {}",
        answer.text
    );
    drop(app);
    desk.drop_schema().await;
}

// ── case 3: X-Forwarded-For ──────────────────────────────────────────────

async fn x_forwarded_for_neither_splits_nor_joins_callers(door: &str) {
    let desk = desk().await;
    let app = desk_app!(desk);
    // one X-Real-IP, a new X-Forwarded-For every time: one caller
    for n in 0..ALLOWED {
        let said = attempt(
            &app,
            door,
            &format!("e{n}"),
            &[
                (XFF, &format!("198.51.100.{n}, 10.0.0.1")),
                (REAL, "203.0.113.1"),
            ],
        )
        .await;
        assert_eq!(said, WRONG, "{door}: attempt {} from 203.0.113.1", n + 1);
    }
    let said = attempt(
        &app,
        door,
        "e-last",
        &[(XFF, "198.51.100.252"), (REAL, "203.0.113.1")],
    )
    .await;
    assert_eq!(
        said, LIMIT,
        "{door}: a new X-Forwarded-For on every request does not walk around the limit"
    );
    // the X-Forwarded-For of the refused request, from another X-Real-IP: another caller
    let said = attempt(
        &app,
        door,
        "e-other",
        &[(XFF, "198.51.100.252"), (REAL, "203.0.113.2")],
    )
    .await;
    assert_eq!(
        said, WRONG,
        "{door}: X-Forwarded-For does not make one caller of two"
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn x_forwarded_for_neither_splits_nor_joins_callers_at_the_staff_door() {
    x_forwarded_for_neither_splits_nor_joins_callers(STAFF_DOOR).await;
}

#[actix_web::test]
async fn x_forwarded_for_neither_splits_nor_joins_callers_at_the_shared_door() {
    x_forwarded_for_neither_splits_nor_joins_callers(SHARED_DOOR).await;
}

// ── case 4: no address ───────────────────────────────────────────────────

/// Six requests that name no address in `X-Real-IP`, each a different way — and the sixth names
/// the bucket itself. `n` keeps the client's own headers different from request to request.
fn nameless(n: usize) -> Vec<(&'static str, String)> {
    match n % 6 {
        0 => vec![],
        1 => vec![(REAL, String::new())],
        2 => vec![(REAL, "  ".to_owned())],
        3 => vec![(CF, format!("198.51.100.{}", n % 250))],
        4 => vec![(XFF, format!("198.51.100.{}", n % 250))],
        _ => vec![(REAL, "unknown".to_owned())],
    }
}

async fn callers_without_an_address_share_one_counter(door: &str, other_door: &str) {
    let desk = desk().await;
    let app = desk_app!(desk);
    for n in 0..ALLOWED {
        let headers = nameless(n);
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let said = attempt(&app, door, &format!("g{n}"), &headers).await;
        assert_eq!(said, WRONG, "{door}: attempt {} ({headers:?})", n + 1);
    }
    for (n, headers) in (0..6).map(|n| (n, nameless(1000 + n))) {
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let said = attempt(&app, door, &format!("h{n}"), &headers).await;
        assert_eq!(
            said, LIMIT,
            "{door}: after 30 attempts without an address, one more is refused however it \
             leaves the address out ({headers:?})"
        );
    }
    let said = attempt(&app, other_door, "i", &[]).await;
    assert_eq!(
        said, LIMIT,
        "{other_door} counts callers without an address in the same counter as {door}"
    );
    // …and that counter is theirs alone
    let said = attempt(&app, door, "j", &[(REAL, "203.0.113.9")]).await;
    assert_eq!(
        said, WRONG,
        "{door}: a caller with an address is not among them"
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn callers_without_an_address_share_one_counter_at_the_staff_door() {
    callers_without_an_address_share_one_counter(STAFF_DOOR, SHARED_DOOR).await;
}

#[actix_web::test]
async fn callers_without_an_address_share_one_counter_at_the_shared_door() {
    callers_without_an_address_share_one_counter(SHARED_DOOR, STAFF_DOOR).await;
}

// ── case 5: the function ─────────────────────────────────────────────────

fn address_of(request: TestRequest) -> String {
    client_address(&request.to_http_request()).to_owned()
}

#[test]
fn the_header_and_the_address_of_nobody_are_these() {
    assert_eq!(CLIENT_ADDRESS_HEADER, "x-real-ip");
    assert_eq!(UNKNOWN_ADDRESS, "unknown");
}

#[test]
fn the_address_is_what_x_real_ip_says() {
    assert_eq!(
        address_of(TestRequest::default().insert_header((REAL, "203.0.113.7"))),
        "203.0.113.7"
    );
    assert_eq!(
        address_of(TestRequest::default().insert_header(("X-Real-IP", "2001:db8::1"))),
        "2001:db8::1",
        "header names are not case-sensitive"
    );
}

#[test]
fn spaces_around_the_address_are_not_part_of_it() {
    for sent in [
        " 203.0.113.7",
        "203.0.113.7 ",
        "\t203.0.113.7\t ",
        "  203.0.113.7  ",
    ] {
        assert_eq!(
            address_of(TestRequest::default().insert_header((REAL, sent))),
            "203.0.113.7",
            "{sent:?}"
        );
    }
}

#[test]
fn a_request_without_the_header_is_from_unknown() {
    assert_eq!(address_of(TestRequest::default()), "unknown");
}

#[test]
fn an_empty_or_blank_header_is_from_unknown() {
    for sent in ["", " ", "   ", "\t", " \t "] {
        assert_eq!(
            address_of(TestRequest::default().insert_header((REAL, sent))),
            "unknown",
            "{sent:?}"
        );
    }
}

#[test]
fn a_header_that_is_not_text_is_from_unknown_and_nothing_panics() {
    let not_text: [&[u8]; 4] = [
        &[0xff, 0xfe],
        &[b'1', b'.', b'2', 0x80, b'3'],
        "ที่อยู่".as_bytes(),
        &[0xc3, 0x28],
    ];
    for bytes in not_text {
        let value = HeaderValue::from_bytes(bytes).expect("a header value may hold these bytes");
        assert_eq!(
            address_of(TestRequest::default().insert_header((REAL, value))),
            "unknown",
            "{bytes:?}"
        );
    }
}

#[test]
fn the_headers_a_client_can_write_are_never_the_address() {
    assert_eq!(
        address_of(TestRequest::default().insert_header((CF, "198.51.100.1"))),
        "unknown"
    );
    assert_eq!(
        address_of(TestRequest::default().insert_header((XFF, "198.51.100.2"))),
        "unknown"
    );
    assert_eq!(
        address_of(
            TestRequest::default()
                .insert_header((CF, "198.51.100.1"))
                .insert_header((XFF, "198.51.100.2"))
                .insert_header((REAL, "203.0.113.7"))
        ),
        "203.0.113.7"
    );
    assert_eq!(
        address_of(
            TestRequest::default()
                .insert_header((CF, "198.51.100.1"))
                .insert_header((XFF, "198.51.100.2"))
                .insert_header((REAL, ""))
        ),
        "unknown",
        "an empty X-Real-IP does not fall back to a header the client wrote"
    );
}

// ── one place ────────────────────────────────────────────────────────────

fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|name| name != "target" && name != "tests")
            {
                rust_sources(&path, found);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// The address is read in one place: no other server source names an address header.
/// A later per-address limit calls `client_address`; it does not read the request itself.
#[test]
fn no_server_source_but_client_address_names_an_address_header() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    rust_sources(&root.join("src"), &mut sources);
    rust_sources(&root.join("crates"), &mut sources);
    assert!(
        sources.len() > 20,
        "the walk found the server's sources: {}",
        sources.len()
    );
    let the_one_place = root.join("src/presentation/http/client_address.rs");
    assert!(
        sources.contains(&the_one_place),
        "{}",
        the_one_place.display()
    );

    let mut offences = Vec::new();
    for path in &sources {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
            .to_lowercase();
        for (number, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for header in ["cf-connecting-ip", "x-forwarded-for", "x-real-ip"] {
                let allowed = header == "x-real-ip" && *path == the_one_place;
                if code.contains(header) && !allowed {
                    offences.push(format!("{}:{}: {header}", path.display(), number + 1));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "an address header is named outside client_address.rs:\n{}",
        offences.join("\n")
    );
}
