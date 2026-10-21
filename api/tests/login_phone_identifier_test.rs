//! #1226 (OR-5c, design `docs/design/blueprint/org-roles-branches.md` §5.3): `SessionService::login` sends a phone to
//! Identity in E.164, whatever separators were typed, and every other identifier unchanged.
//!
//! Written before the implementation by the test-writer (claude/709ec749, ordered by the xchange BE lane controller
//! claude/ad05bf8d). Do not edit, delete or weaken these tests; add your own in another file.
//!
//! Every test asserts what the fake Identity's `/auth/login` received (#1226 §8). The rule pinned (§3):
//! - strip spaces, `-`, `(` and `)`;
//! - what is left is a phone when it is `0` followed by 8 or 9 more digits (a Thai number), or `+` followed by
//!   digits, and it is then sent as `xc_kernel::pii::normalize(Phone, …)` (`0…` → `+66…`);
//! - anything else (a username, an email, `0123`, an 11-digit `0…`) is sent exactly as typed.
//!
//! The rows whose answer is "unchanged" hold on main as well (main sends every identifier as typed): they pin that the
//! fix does not normalise what is not a phone.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`). Without it they
//! FAIL; none skips.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::{Value, json};

const PASSWORD: &str = "pw-from-the-browser-01";

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// Signs in with `typed` against a fake Identity that refuses every login (so no user has to exist), and returns the
/// one `/auth/login` body Identity received.
async fn sent_for(typed: &str) -> Value {
    let (desk, identity, app) = setup().await;
    identity.answer("/auth/login", 401, r#"{"error":"invalid_credentials"}"#);
    let _ = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": typed, "password": PASSWORD }),
        ))
        .await;
    let received = identity.received("/auth/login");
    assert_eq!(
        received.len(),
        1,
        "{typed:?}: one call to Identity's /auth/login"
    );
    let body = received[0].json();
    assert_eq!(
        body["password"],
        json!(PASSWORD),
        "{typed:?}: the password is sent as typed"
    );
    desk.drop_schema().await;
    body
}

async fn assert_sent(typed: &str, expected: &str) {
    let body = sent_for(typed).await;
    assert_eq!(
        body["identifier"],
        json!(expected),
        "typed {typed:?}: Identity must receive {expected:?}"
    );
}

// ── §8: a Thai phone typed the local way → E.164 ──────────────────────────────────────────

/// The owner's report (#1226 §1): `0812341688` was refused while `+66812341688` signed in. Typed the local way, the
/// phone reaches Identity in E.164 and the person signs in as their own user.
#[actix_web::test]
async fn a_local_mobile_number_is_sent_in_e164_and_signs_in() {
    let (desk, identity, app) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Somchai",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);

    let reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "0812341688", "password": PASSWORD }),
        ))
        .await;

    let received = identity.received("/auth/login");
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].json()["identifier"], json!("+66812341688"));
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["user"]["id"], json!(user), "{}", reply.text);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_local_mobile_number_with_dashes_is_sent_in_e164() {
    assert_sent("081-234-1688", "+66812341688").await;
}

#[actix_web::test]
async fn a_local_mobile_number_with_spaces_is_sent_in_e164() {
    assert_sent("081 234 1688", "+66812341688").await;
}

#[actix_web::test]
async fn a_local_mobile_number_with_brackets_is_sent_in_e164() {
    assert_sent("(081) 234-1688", "+66812341688").await;
}

/// A 9-digit Thai number (`0` and 8 more digits, a Bangkok landline).
#[actix_web::test]
async fn a_nine_digit_landline_is_sent_in_e164() {
    assert_sent("021234567", "+6621234567").await;
}

// ── §8: E.164 typed as such ───────────────────────────────────────────────────────────────

#[actix_web::test]
async fn an_e164_number_with_spaces_is_sent_without_them() {
    assert_sent("+66 81 234 1688", "+66812341688").await;
}

#[actix_web::test]
async fn an_e164_number_is_sent_unchanged() {
    assert_sent("+66812341688", "+66812341688").await;
}

/// §3: `+` followed by digits is a phone, whatever the country. A non-Thai number loses its separators and keeps its
/// own country code: it is never treated as a username, and never rewritten as `+66…`.
#[actix_web::test]
async fn a_non_thai_international_number_keeps_its_country_code_without_separators() {
    assert_sent("+1 (415) 555-0100", "+14155550100").await;
}

#[actix_web::test]
async fn a_non_thai_international_number_with_spaces_is_sent_without_them() {
    assert_sent("+44 20 7946 0958", "+442079460958").await;
}

// ── §8, §9: not a phone → unchanged ───────────────────────────────────────────────────────

#[actix_web::test]
async fn a_username_is_sent_unchanged() {
    assert_sent("somchai", "somchai").await;
}

#[actix_web::test]
async fn an_email_is_sent_unchanged() {
    assert_sent("a@b.co", "a@b.co").await;
}

/// All digits, but too short to be a Thai number: a username, sent as typed.
#[actix_web::test]
async fn a_short_all_digit_username_is_sent_unchanged() {
    assert_sent("0123", "0123").await;
}

/// `0` and 10 more digits: not a Thai number, sent as typed.
#[actix_web::test]
async fn an_eleven_digit_number_is_sent_unchanged() {
    assert_sent("08123456789", "08123456789").await;
}

/// A username that merely contains digits and separators is not a phone either.
#[actix_web::test]
async fn a_username_with_digits_and_a_dash_is_sent_unchanged() {
    assert_sent("som-0812", "som-0812").await;
}

// ── §9: a wrong password still gives the same refusal ─────────────────────────────────────

/// Identity answers 401 `invalid_credentials`: the reply is the sign-in refusal it always was
/// (`identity_signin_test` `each_identity_error_code_maps_to_its_sentence`), and the phone was still sent in E.164.
#[actix_web::test]
async fn a_wrong_password_with_a_local_phone_keeps_the_same_refusal() {
    let (desk, identity, app) = setup().await;
    identity.answer("/auth/login", 401, r#"{"error":"invalid_credentials"}"#);

    let reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "081-234-1688", "password": "not-the-password" }),
        ))
        .await;

    assert_eq!(reply.status, 401, "{}", reply.text);
    assert_eq!(
        reply.refusal(),
        (
            "invalidCredentials".to_owned(),
            "ชื่อผู้ใช้หรือรหัสผ่านไม่ถูกต้อง".to_owned()
        ),
        "{}",
        reply.text
    );
    assert!(reply.sets_no_session(), "no session on a refusal");
    let received = identity.received("/auth/login");
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].json()["identifier"], json!("+66812341688"));
    desk.drop_schema().await;
}
