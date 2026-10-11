//! Ticket #1255 (RD-0, `docs/design/blueprint/rate-display.md` §4.1a, D2, D13): `GET /api/public/host`
//! answers which organisation the request's host belongs to, from `organization_hosts`.
//!
//! Written before the implementation by the test-writer claude/07d3765f, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Every §4.1a row is a test here, and each test inserts its own `organization_hosts` rows:
//! - a host with a row → `200 {"org":"<code>"}`, `Cache-Control: no-store`;
//! - the host is lower-cased and loses the scheme's default port (`:443` with `X-Forwarded-Proto: https`);
//! - a non-default port (`:8443`) is kept;
//! - no row, or no `Host` → `404`, byte-identical to `public::not_found_response()`;
//! - `X-Forwarded-Host` is not read;
//! - a session cookie or an `Authorization` header is ignored, as on every public route.
//!
//! The `request_host` tests pin the same rule on the function itself (§3 [required]); they need no
//! database. The HTTP tests need `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod quote_common;

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, test as atest};
use quote_common::TestDb;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::extractors::request_host::request_host;
use xchange::presentation::http::public::not_found_response;

const ROUTE: &str = "/api/public/host";
/// The organisation every migrated database is seeded with (`20261011100000_organizations.sql`).
const XC_ID: &str = "00000000-0000-0000-0000-000000000001";
/// A second organisation, made by `second_organisation`.
const YY_ID: &str = "00000000-0000-0000-0000-0000000000b2";

struct Answer {
    status: StatusCode,
    cache_control: Option<String>,
    body: Vec<u8>,
}

/// `GET /api/public/host` through the whole route table, with exactly these headers (no `Host` unless given).
async fn get(db: &TestDb, headers: &[(&str, &str)]) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let app =
        atest::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
            .await;
    let mut req = atest::TestRequest::get().uri(ROUTE);
    for (name, value) in headers {
        req = req.insert_header((*name, *value));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let cache_control = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().expect("an ASCII header").to_owned());
    let body = atest::read_body(resp).await.to_vec();
    Answer {
        status,
        cache_control,
        body,
    }
}

/// The shared public 404, as bytes.
async fn not_found_body() -> Vec<u8> {
    actix_web::body::to_bytes(not_found_response().into_body())
        .await
        .expect("the 404 body")
        .to_vec()
}

async fn assert_not_found(answer: &Answer, what: &str) {
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{what}");
    assert_eq!(
        answer.body,
        not_found_body().await,
        "{what}: the body is byte-identical to public::not_found_response(): {}",
        String::from_utf8_lossy(&answer.body)
    );
}

fn assert_org(answer: &Answer, code: &str, what: &str) {
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{what}: {}",
        String::from_utf8_lossy(&answer.body)
    );
    let body: serde_json::Value = serde_json::from_slice(&answer.body)
        .unwrap_or_else(|e| panic!("{what}: a JSON body ({e})"));
    assert_eq!(body, serde_json::json!({ "org": code }), "{what}");
}

async fn host_row(db: &TestDb, host: &str, organization_id: &str) {
    db.exec(&format!(
        "INSERT INTO organization_hosts (host, organization_id) VALUES ('{host}', '{organization_id}')"
    ))
    .await;
}

async fn second_organisation(db: &TestDb) {
    db.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{YY_ID}', 'YY', NULL, 'Asia/Bangkok', now())"
    ))
    .await;
}

// ── §4.1a, through the route ─────────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn a_host_with_a_row_answers_its_organisation_with_no_store() {
    let db = TestDb::start().await;
    host_row(&db, "rates.example.test", XC_ID).await;

    let answer = get(&db, &[("Host", "rates.example.test")]).await;
    assert_org(&answer, "XC", "Host: rates.example.test");
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"));
    db.finish().await;
}

#[actix_web::test]
async fn an_upper_case_host_with_the_default_https_port_finds_the_same_row() {
    let db = TestDb::start().await;
    host_row(&db, "rates.example.test", XC_ID).await;

    let answer = get(
        &db,
        &[
            ("Host", "RATES.Example.TEST:443"),
            ("X-Forwarded-Proto", "https"),
        ],
    )
    .await;
    assert_org(&answer, "XC", "Host: RATES.Example.TEST:443, https");

    let answer = get(&db, &[("Host", "Rates.Example.Test:80")]).await;
    assert_org(&answer, "XC", "Host: Rates.Example.Test:80, http");
    db.finish().await;
}

#[actix_web::test]
async fn a_non_default_port_is_kept_in_the_lookup() {
    let db = TestDb::start().await;
    second_organisation(&db).await;
    host_row(&db, "rates.example.test", XC_ID).await;

    let answer = get(
        &db,
        &[
            ("Host", "rates.example.test:8443"),
            ("X-Forwarded-Proto", "https"),
        ],
    )
    .await;
    assert_not_found(&answer, "Host: rates.example.test:8443 with no row for it").await;

    host_row(&db, "rates.example.test:8443", YY_ID).await;
    let answer = get(
        &db,
        &[
            ("Host", "rates.example.test:8443"),
            ("X-Forwarded-Proto", "https"),
        ],
    )
    .await;
    assert_org(
        &answer,
        "YY",
        "Host: rates.example.test:8443 with its own row",
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_host_with_no_row_is_the_shared_404() {
    let db = TestDb::start().await;
    host_row(&db, "rates.example.test", XC_ID).await;

    let answer = get(&db, &[("Host", "other.example.test")]).await;
    assert_not_found(&answer, "Host: other.example.test").await;
    db.finish().await;
}

#[actix_web::test]
async fn no_host_header_is_the_shared_404() {
    let db = TestDb::start().await;
    host_row(&db, "rates.example.test", XC_ID).await;

    let answer = get(&db, &[]).await;
    assert_not_found(&answer, "no Host").await;
    db.finish().await;
}

#[actix_web::test]
async fn x_forwarded_host_is_not_read() {
    let db = TestDb::start().await;
    host_row(&db, "rates.example.test", XC_ID).await;

    let answer = get(
        &db,
        &[
            ("Host", "other.example.test"),
            ("X-Forwarded-Host", "rates.example.test"),
        ],
    )
    .await;
    assert_not_found(&answer, "X-Forwarded-Host names the row, Host does not").await;
    db.finish().await;
}

#[actix_web::test]
async fn a_session_cookie_and_an_authorization_header_are_ignored() {
    let db = TestDb::start().await;
    host_row(&db, "rates.example.test", XC_ID).await;
    let credentials = [
        ("Cookie", "xc_session=not-a-session; xc_staff=nor-this"),
        ("Authorization", "Bearer not-a-token"),
    ];

    let mut headers = vec![("Host", "rates.example.test")];
    headers.extend(credentials);
    let answer = get(&db, &headers).await;
    assert_org(&answer, "XC", "a known host with bad credentials");
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"));

    let mut headers = vec![("Host", "other.example.test")];
    headers.extend(credentials);
    let answer = get(&db, &headers).await;
    assert_not_found(&answer, "an unknown host with bad credentials").await;
    db.finish().await;
}

// ── §3: request_host itself ──────────────────────────────────────────────────────────────────────────

/// `X-Forwarded-Proto: https`, the one way `request_host` reads the scheme.
const HTTPS: (&str, &str) = ("X-Forwarded-Proto", "https");

/// The headers sent, and the host `request_host` must answer.
type Case = (
    &'static [(&'static str, &'static str)],
    Option<&'static str>,
);

fn host_of(headers: &[(&str, &str)]) -> Option<String> {
    let mut req = atest::TestRequest::get().uri(ROUTE);
    for (name, value) in headers {
        req = req.insert_header((*name, *value));
    }
    request_host(&req.to_http_request())
}

#[test]
fn request_host_lower_cases_and_drops_only_the_schemes_default_port() {
    let cases: [Case; 7] = [
        (
            &[("Host", "RATES.Example.TEST")],
            Some("rates.example.test"),
        ),
        (
            &[("Host", "RATES.Example.TEST:443"), HTTPS],
            Some("rates.example.test"),
        ),
        (
            &[("Host", "rates.example.test:80")],
            Some("rates.example.test"),
        ),
        // the other scheme's default port is not this scheme's: kept
        (
            &[("Host", "rates.example.test:443")],
            Some("rates.example.test:443"),
        ),
        (
            &[("Host", "rates.example.test:80"), HTTPS],
            Some("rates.example.test:80"),
        ),
        // a non-default port is kept
        (
            &[("Host", "rates.example.test:8443"), HTTPS],
            Some("rates.example.test:8443"),
        ),
        (
            &[("Host", "Rates.Example.Test:8080")],
            Some("rates.example.test:8080"),
        ),
    ];
    for (headers, expected) in cases {
        assert_eq!(host_of(headers).as_deref(), expected, "{headers:?}");
    }
}

#[test]
fn request_host_is_none_without_a_host_and_never_reads_x_forwarded_host() {
    assert_eq!(host_of(&[]), None, "no Host");
    assert_eq!(
        host_of(&[("X-Forwarded-Host", "rates.example.test")]),
        None,
        "X-Forwarded-Host alone"
    );
    assert_eq!(
        host_of(&[
            ("Host", "other.example.test"),
            ("X-Forwarded-Host", "rates.example.test"),
        ])
        .as_deref(),
        Some("other.example.test"),
        "Host wins; X-Forwarded-Host is not read"
    );
}
