//! Written before the implementation (ticket #161). Do not edit, delete or weaken.
//!
//! The one same-origin rule, v2's: the request's own origin is the scheme (`https` only when
//! `X-Forwarded-Proto` is exactly `https`) and the `Host` header lower-cased, without the
//! scheme's default port; the `Origin` header is compared with it **as sent**; and the marker
//! `X-Xchange-Request: 1` is required.
//!
//! The eight rows of `V2_ROWS` were measured through v2's own `handle()` (`xchange-online@3ffc5a5`,
//! `POST /api/rpc`, owner session, marker sent) by claude/be518a0f, 2026-10-07 — ticket #124,
//! comment 6022240969. The comparison in v2 is `origin !== new URL(request.url).origin`
//! (`server.mjs:5542`, `7667`, `8050`); `URL` normalises its own side only.
//!
//! The function is tested directly, then through one route of each copy the ticket replaces
//! that can be reached with no session and no database: `/api/auth` (the copy of
//! `extractors/actor.rs`) and `/api/staff/upload` (the copy of `http/files.rs`, which checks the
//! origin before it asks who is calling). Nothing here needs `TEST_DATABASE_URL`.

mod common;

use std::sync::Arc;

use actix_web::App;
use actix_web::http::StatusCode;
use actix_web::test as http_test;
use serde_json::{Value, json};
use xchange::bootstrap::{AppState, configure};
use xchange::presentation::extractors::origin::same_origin;

/// `(X-Forwarded-Proto, Host, Origin, v2 accepted)` — the eight measured rows, in the order of
/// the comment on #124.
const V2_ROWS: [(Option<&str>, &str, &str, bool); 8] = [
    (None, "desk.test", "http://desk.test", true),
    (None, "DESK.TEST", "http://desk.test", true),
    (None, "desk.test", "http://DESK.TEST", false),
    (None, "desk.test:80", "http://desk.test", true),
    (None, "desk.test", "http://desk.test:80", false),
    (Some("https"), "desk.test:443", "https://desk.test", true),
    (None, "desk.test:8080", "http://desk.test:8080", true),
    (None, "desk.test:8080", "http://desk.test", false),
];

fn request(
    proto: Option<&str>,
    host: Option<&str>,
    origin: Option<&str>,
    marker: Option<&str>,
) -> http_test::TestRequest {
    let mut req = http_test::TestRequest::post();
    if let Some(proto) = proto {
        req = req.insert_header(("X-Forwarded-Proto", proto));
    }
    if let Some(host) = host {
        req = req.insert_header(("Host", host));
    }
    if let Some(origin) = origin {
        req = req.insert_header(("Origin", origin));
    }
    if let Some(marker) = marker {
        req = req.insert_header(("X-Xchange-Request", marker));
    }
    req
}

fn judged(
    proto: Option<&str>,
    host: Option<&str>,
    origin: Option<&str>,
    marker: Option<&str>,
) -> bool {
    same_origin(&request(proto, host, origin, marker).to_http_request())
}

#[test]
fn the_eight_measured_rows_are_answered_as_v2_answered_them() {
    for (proto, host, origin, v2) in V2_ROWS {
        assert_eq!(
            judged(proto, Some(host), Some(origin), Some("1")),
            v2,
            "X-Forwarded-Proto {proto:?}, Host {host}, Origin {origin}"
        );
    }
}

#[test]
fn only_the_host_side_is_normalised() {
    let cases: [(Option<&str>, &str, &str, bool); 7] = [
        // the default port of the *other* scheme is a port like any other
        (None, "desk.test:443", "http://desk.test", false),
        (None, "desk.test:443", "http://desk.test:443", true),
        (Some("https"), "desk.test:80", "https://desk.test", false),
        (Some("https"), "desk.test:80", "https://desk.test:80", true),
        // upper case and a default port together
        (Some("https"), "Desk.Test:443", "https://desk.test", true),
        // the Origin keeps its case and its port
        (Some("https"), "desk.test", "https://Desk.test", false),
        (Some("https"), "desk.test", "https://desk.test:443", false),
    ];
    for (proto, host, origin, expected) in cases {
        assert_eq!(
            judged(proto, Some(host), Some(origin), Some("1")),
            expected,
            "X-Forwarded-Proto {proto:?}, Host {host}, Origin {origin}"
        );
    }
}

#[test]
fn the_scheme_is_https_only_when_the_proxy_says_exactly_https() {
    let cases: [(Option<&str>, &str, bool); 6] = [
        (Some("https"), "https://desk.test", true),
        (Some("https"), "http://desk.test", false),
        (Some("HTTPS"), "http://desk.test", true),
        (Some("http"), "http://desk.test", true),
        (None, "http://desk.test", true),
        (None, "https://desk.test", false),
    ];
    for (proto, origin, expected) in cases {
        assert_eq!(
            judged(proto, Some("desk.test"), Some(origin), Some("1")),
            expected,
            "X-Forwarded-Proto {proto:?}, Origin {origin}"
        );
    }
}

#[test]
fn the_marker_and_the_origin_header_are_required() {
    let host = Some("desk.test");
    let origin = Some("http://desk.test");
    assert!(judged(None, host, origin, Some("1")));
    assert!(!judged(None, host, origin, None), "no marker");
    assert!(!judged(None, host, origin, Some("yes")), "another marker");
    assert!(
        !judged(None, host, origin, Some("1 ")),
        "the marker with a space"
    );
    assert!(!judged(None, host, None, Some("1")), "no Origin");
    assert!(!judged(None, host, Some("null"), Some("1")), "Origin null");
}

#[test]
fn a_request_with_no_host_is_refused() {
    // Ticket #161 §7: the copies disagree today (two use `localhost`, one refuses) and v2 was not
    // measured here; the one rule refuses.
    assert!(!judged(None, None, Some("http://localhost"), Some("1")));
    assert!(!judged(None, None, Some("http://"), Some("1")));
}

// ── through the routes ───────────────────────────────────────────────────

async fn send(
    proto: Option<&str>,
    host: &str,
    origin: &str,
    path: &str,
    body: Value,
) -> (StatusCode, String) {
    let state = AppState::new(Arc::new(common::settings(&[])), common::unreachable_pool());
    let app = http_test::init_service(App::new().configure(move |cfg| configure(cfg, state))).await;
    let req = request(proto, Some(host), Some(origin), Some("1"))
        .uri(path)
        .insert_header(("Content-Type", "application/json"))
        .set_payload(body.to_string())
        .to_request();
    let response = http_test::call_service(&app, req).await;
    let status = response.status();
    let bytes = http_test::read_body(response).await;
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// What each route answers when its origin check refuses, with no session.
const AUTH_REFUSED: &str = r#"{"error":"คำขอไม่ถูกต้อง"}"#;
const STAFF_UPLOAD_REFUSED: &str = r#"{"error":"คำขอไม่ถูกต้อง"}"#;

#[actix_web::test]
async fn the_copy_of_extractors_actor_answers_as_v2_on_api_auth() {
    for (proto, host, origin, v2) in V2_ROWS {
        let (status, body) = send(
            proto,
            host,
            origin,
            "/api/auth",
            json!({ "action": "status" }),
        )
        .await;
        let what = format!("/api/auth, X-Forwarded-Proto {proto:?}, Host {host}, Origin {origin}");
        if v2 {
            // nobody is signed in: the status answer, no database asked
            assert_eq!(status, StatusCode::OK, "{what}: {body}");
            assert_eq!(
                serde_json::from_str::<Value>(&body).expect("JSON"),
                json!({ "setupRequired": false, "user": null }),
                "{what}"
            );
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
            assert_eq!(body, AUTH_REFUSED, "{what}");
        }
    }
}

#[actix_web::test]
async fn the_copy_of_http_files_answers_as_v2_on_api_staff_upload() {
    for (proto, host, origin, v2) in V2_ROWS {
        let (status, body) = send(proto, host, origin, "/api/staff/upload", json!({})).await;
        let what =
            format!("/api/staff/upload, X-Forwarded-Proto {proto:?}, Host {host}, Origin {origin}");
        if v2 {
            // past the origin check: nobody is signed in, so it is the session's refusal
            assert_ne!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{what}: {body}");
        } else {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
            assert_eq!(body, STAFF_UPLOAD_REFUSED, "{what}");
        }
    }
}
