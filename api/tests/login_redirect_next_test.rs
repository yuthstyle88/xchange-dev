//! Ticket #1234 (design row S-10, `docs/design/xchange-v3-wave3-staff.md`): the server's sign-in redirect carries
//! the page the visitor asked for, as `next`, so S-9's sign-in screen can send them back to it.
//!
//! Written before the implementation by the test-writer claude/07d3765f, on the lane controller claude/ad05bf8d's
//! order. Do not edit, delete or weaken.
//!
//! - §8: `Location` is `{origin}/login?next=<the path and query, percent-encoded once>`.
//!   Row 1 is checked on pages gated on main today (`/desk` through `pages.rs`, the receipt route), per the lane
//!   controller's ruling on #1234: `/staff` is gated by S-3 #575, which then uses the same `to_login`.
//! - §9: decoding `next` once gives back the path and query exactly. `next` never carries a scheme or a host,
//!   whatever `Host` / `X-Forwarded-*` the request sends. Each of `to_login`'s four callers answers so.
//!
//! The four callers, and the request that reaches each one:
//! - `pages.rs` `devices_receipt_handler`: `GET /devices/receipt?…`;
//! - `pages.rs` page specs with `who_signed_in` (not a screen): `GET /desk`;
//! - `screens.rs` `Gate::SignedIn`: `GET /staff-guide`;
//! - `screens.rs` `Gate::Branch`: `GET /receipt-sample`.
//!
//! The screen table is read from `tests/fixtures/ui/built` (as `ui_assets_test.rs` does), so which screens exist
//! does not depend on whether `web/dist` was built. No test here needs a database.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test, web};
use common::{FakeDatabase, settings, state_with};
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::screens::ScreenTable;
use xchange::presentation::test_callers;

const HOST: &str = "xc.test";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Nobody signed in, or one FRONT staff member of `MAIN`.
struct FixedSessions {
    signed_in: bool,
}

impl SessionReader for FixedSessions {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let staff = self
            .signed_in
            .then(|| test_callers::staff("employee-7", &[Role::Teller], BranchId::new("MAIN")));
        Box::pin(async move { test_callers::callers(None, staff) })
    }
}

struct Answer {
    status: StatusCode,
    location: Option<String>,
}

/// `GET uri` with `Host: xc.test` plus `headers`, signed out unless `signed_in`.
async fn get(signed_in: bool, uri: &str, headers: &[(&str, &str)]) -> Answer {
    let screens = ScreenTable::from_dist(&fixtures().join("ui/built"))
        .unwrap_or_else(|e| panic!("the fixture dist must load: {e:?}"));
    let web_dir = fixtures().join("pages/web");
    let state = state_with(
        settings(&[("APP_WEB__DIR", web_dir.to_str().expect("a UTF-8 path"))]),
        FakeDatabase::Up,
    );
    let reader: Arc<dyn SessionReader> = Arc::new(FixedSessions { signed_in });
    let app = test::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure_with_screens(cfg, state, screens))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = test::TestRequest::get().uri(uri);
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        req = req.insert_header((header::HOST, HOST));
    }
    for (name, value) in headers {
        req = req.insert_header((*name, *value));
    }
    let resp = test::call_service(&app, req.to_request()).await;
    Answer {
        status: resp.status(),
        location: resp
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().expect("a text Location").to_owned()),
    }
}

/// The `next` of a sign-in `Location`, still encoded; panics when the answer is not a sign-in redirect.
fn next_of(answer: &Answer, what: &str) -> String {
    assert_eq!(
        answer.status,
        StatusCode::SEE_OTHER,
        "{what}: a sign-in redirect"
    );
    let location = answer
        .location
        .as_deref()
        .unwrap_or_else(|| panic!("{what}: a Location"));
    let (_, query) = location
        .split_once("/login?")
        .unwrap_or_else(|| panic!("{what}: Location {location:?} is /login with a query"));
    query
        .strip_prefix("next=")
        .unwrap_or_else(|| panic!("{what}: Location {location:?} carries next= first"))
        .to_owned()
}

/// One percent-decoding pass, as S-9's sign-in screen does.
fn decode_once(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).expect("ascii hex");
            out.push(u8::from_str_radix(hex, 16).expect("two hex digits"));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("UTF-8 after decoding")
}

// ── §8 ─────────────────────────────────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_query_is_kept_and_encoded_exactly_once() {
    let answer = get(false, "/devices?x=1&y=a%20b", &[]).await;
    assert_eq!(
        answer.status,
        StatusCode::SEE_OTHER,
        "GET /devices signed out"
    );
    assert_eq!(
        answer.location.as_deref(),
        Some("http://xc.test/login?next=%2Fdevices%3Fx%3D1%26y%3Da%2520b")
    );
    assert_eq!(
        decode_once(&next_of(&answer, "/devices?x=1&y=a%20b")),
        "/devices?x=1&y=a%20b",
        "one decode gives back the path and query exactly"
    );
}

#[actix_web::test]
async fn a_signed_in_visitor_is_not_redirected() {
    for path in ["/devices", "/desk", "/staff-guide", "/receipt-sample"] {
        let answer = get(true, path, &[]).await;
        assert_ne!(
            answer.status,
            StatusCode::SEE_OTHER,
            "GET {path} signed in: {:?}",
            answer.location
        );
        assert!(
            answer
                .location
                .as_deref()
                .is_none_or(|l| !l.contains("/login")),
            "GET {path} signed in is not sent to sign in: {:?}",
            answer.location
        );
    }
}

#[actix_web::test]
async fn the_login_page_itself_is_not_redirected() {
    let answer = get(false, "/login", &[]).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "GET /login is an anyone page"
    );
    assert!(answer.location.is_none(), "{:?}", answer.location);
}

// ── §9 ─────────────────────────────────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn next_never_carries_a_scheme_or_host_whatever_the_request_claims() {
    let forged: [&[(&str, &str)]; 3] = [
        &[("Host", "evil.test")],
        &[("X-Forwarded-Host", "evil.test")],
        &[
            ("Host", "evil.test"),
            ("X-Forwarded-Host", "evil2.test"),
            ("X-Forwarded-Proto", "https"),
        ],
    ];
    for headers in forged {
        let answer = get(false, "/devices?x=1", headers).await;
        let next = decode_once(&next_of(&answer, &format!("{headers:?}")));
        assert_eq!(
            next, "/devices?x=1",
            "{headers:?}: next is the path and query only"
        );
        assert!(
            next.starts_with('/') && !next.starts_with("//") && !next.contains("://"),
            "{headers:?}: next {next:?} has no scheme or host"
        );
        assert!(
            !next.contains("evil"),
            "{headers:?}: next {next:?} names no host from the request"
        );
    }
}

/// `GET path` signed out answers `303` to `http://xc.test/login?next=<encoded>`.
async fn assert_sends_next(path: &str, encoded: &str) {
    let answer = get(false, path, &[]).await;
    assert_eq!(
        answer.status,
        StatusCode::SEE_OTHER,
        "GET {path} signed out"
    );
    assert_eq!(
        answer.location.as_deref(),
        Some(format!("http://xc.test/login?next={encoded}").as_str()),
        "GET {path} signed out"
    );
}

/// `pages.rs` `devices_receipt_handler`.
#[actix_web::test]
async fn the_receipt_preview_caller_sends_next() {
    assert_sends_next(
        "/devices/receipt?branch=MAIN&ref=R1",
        "%2Fdevices%2Freceipt%3Fbranch%3DMAIN%26ref%3DR1",
    )
    .await;
}

/// `pages.rs`: a page spec with `who_signed_in` that is not a screen.
#[actix_web::test]
async fn the_signed_in_page_spec_caller_sends_next() {
    assert_sends_next("/desk", "%2Fdesk").await;
}

/// `screens.rs` `Gate::SignedIn`.
#[actix_web::test]
async fn the_signed_in_screen_gate_caller_sends_next() {
    assert_sends_next("/staff-guide", "%2Fstaff-guide").await;
}

/// `screens.rs` `Gate::Branch`.
#[actix_web::test]
async fn the_branch_screen_gate_caller_sends_next() {
    assert_sends_next("/receipt-sample", "%2Freceipt-sample").await;
}
