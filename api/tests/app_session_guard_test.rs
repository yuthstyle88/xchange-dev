//! APP-4 (#994) — the anonymous app session guard, `api/src/presentation/extractors/app_session.rs`:
//! the `xc_app_sid` cookie (`HttpOnly; Secure; SameSite=Lax`, only its SHA-256 stored) and the
//! per-session limit on draft writes.
//! Written before the implementation by the ticket author's delegate (claude/dell-claude-3). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/customer-app.md` P7, §4.3 (*"the anonymous session is rate-limited
//! … M draft writes per minute → `rateLimited`"*, [proposed] M = 20, look only). Ticket #994 §7, §8.
//!
//! The extractor is exercised on a test route of this file only; wiring it into the app commands is
//! the ticket's route work, tested once OR-4b #767 has merged (commented on #994).
//!
//! Signatures this test assumes (the scaffold commit before this one): `AppSessionCookie:
//! FromRequest` with `session() -> &AppSession` and `issued_cookie() -> Option<Cookie<'static>>`;
//! `session_cookie(&AppSessionId) -> Cookie<'static>`; `AppSessionGuard::new(per_minute)` and
//! `admit_draft_write(&self, &SessionHash, now) -> Result<(), V3Error>`; `DRAFT_WRITES_PER_MINUTE`.

use actix_web::cookie::{Cookie, SameSite};
use actix_web::http::StatusCode;
use actix_web::test as atest;
use actix_web::{App, HttpResponse, web};
use chrono::{Duration, TimeZone, Utc};
use sha2::{Digest, Sha256};
use xc_app::domain::app_session::{AppSessionId, SessionHash};
use xchange::presentation::extractors::app_session::{
    AppSessionCookie, AppSessionGuard, DRAFT_WRITES_PER_MINUTE, session_cookie,
};

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn echo(session: AppSessionCookie) -> HttpResponse {
    let mut response = HttpResponse::Ok().body(session.session().hash.as_str().to_owned());
    if let Some(cookie) = session.issued_cookie() {
        response.add_cookie(&cookie).expect("a valid cookie");
    }
    response
}

/// (status, Set-Cookie headers, body) of one request to the test route.
async fn call(cookie: Option<&str>) -> (StatusCode, Vec<String>, String) {
    let app = atest::init_service(App::new().route("/app-session", web::post().to(echo))).await;
    let mut req = atest::TestRequest::post().uri("/app-session");
    if let Some(value) = cookie {
        req = req.cookie(Cookie::new("xc_app_sid", value.to_owned()));
    }
    let res = atest::call_service(&app, req.to_request()).await;
    let status = res.status();
    let set_cookies = res
        .headers()
        .get_all(actix_web::http::header::SET_COOKIE)
        .map(|v| v.to_str().expect("ascii").to_owned())
        .collect();
    let body = String::from_utf8(atest::read_body(res).await.to_vec()).expect("utf-8");
    (status, set_cookies, body)
}

fn issued_value(set_cookies: &[String]) -> String {
    assert_eq!(set_cookies.len(), 1, "one Set-Cookie: {set_cookies:?}");
    let cookie = Cookie::parse(set_cookies[0].clone()).expect("a cookie");
    assert_eq!(cookie.name(), "xc_app_sid");
    cookie.value().to_owned()
}

#[actix_web::test]
async fn a_request_without_the_cookie_is_issued_one_httponly_secure_lax() {
    let (status, set_cookies, body) = call(None).await;

    assert_eq!(status, StatusCode::OK);
    let cookie =
        Cookie::parse(set_cookies.first().expect("a Set-Cookie").clone()).expect("a cookie");
    assert_eq!(cookie.name(), "xc_app_sid");
    assert_eq!(cookie.http_only(), Some(true));
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(
        body,
        sha256_hex(cookie.value()),
        "the session's hash is the SHA-256 of the cookie id"
    );
}

#[actix_web::test]
async fn a_request_with_its_cookie_keeps_its_session_and_is_issued_none() {
    let (_, first, first_hash) = call(None).await;
    let value = issued_value(&first);

    let (status, second, second_hash) = call(Some(&value)).await;

    assert_eq!(status, StatusCode::OK);
    assert!(second.is_empty(), "no new cookie: {second:?}");
    assert_eq!(second_hash, first_hash);
}

#[actix_web::test]
async fn a_request_with_a_malformed_cookie_is_issued_a_new_session() {
    let (status, set_cookies, body) = call(Some("not a session id!")).await;

    assert_eq!(status, StatusCode::OK);
    let value = issued_value(&set_cookies);
    assert_ne!(value, "not a session id!");
    assert_eq!(body, sha256_hex(&value));
}

#[test]
fn the_session_cookie_is_httponly_secure_lax_on_the_whole_site() {
    let id = AppSessionId::generate();

    let cookie = session_cookie(&id);

    assert_eq!(cookie.name(), "xc_app_sid");
    assert_eq!(cookie.value(), id.as_str());
    assert_eq!(cookie.http_only(), Some(true));
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(cookie.path(), Some("/"));
}

fn instant() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_791_288_000, 0)
        .single()
        .expect("a valid instant")
}

fn hash(n: u8) -> SessionHash {
    SessionHash::from_stored(format!("{n:064x}"))
}

#[test]
fn twenty_draft_writes_a_minute_is_the_proposed_limit() {
    assert_eq!(DRAFT_WRITES_PER_MINUTE, 20);
}

#[test]
fn the_twenty_first_draft_write_in_a_minute_is_rate_limited() {
    let guard = AppSessionGuard::new(DRAFT_WRITES_PER_MINUTE);
    let session = hash(1);

    for n in 0..20 {
        guard
            .admit_draft_write(&session, instant())
            .unwrap_or_else(|e| panic!("write {} refused: {}", n + 1, e.code));
    }
    let refused = guard
        .admit_draft_write(&session, instant())
        .expect_err("the 21st write in one minute");

    assert_eq!(refused.code, "rateLimited");
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
}

#[test]
fn another_session_is_not_limited_by_the_first() {
    let guard = AppSessionGuard::new(DRAFT_WRITES_PER_MINUTE);
    for _ in 0..21 {
        let _ = guard.admit_draft_write(&hash(1), instant());
    }

    assert!(guard.admit_draft_write(&hash(2), instant()).is_ok());
}

#[test]
fn writes_are_admitted_again_once_the_minute_has_passed() {
    let guard = AppSessionGuard::new(DRAFT_WRITES_PER_MINUTE);
    for _ in 0..21 {
        let _ = guard.admit_draft_write(&hash(3), instant());
    }

    assert!(
        guard
            .admit_draft_write(&hash(3), instant() + Duration::seconds(121))
            .is_ok()
    );
}
