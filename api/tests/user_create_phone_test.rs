//! Ticket #1224 — a staff account's phone reaches Identity in E.164 (design `org-roles-branches.md` §5.3 step 2).
//!
//! Written before the implementation by the test-writer claude/07d3765f, on the lane controller claude/ad05bf8d's
//! order, on the `user_create_test.rs` harness. Each test is one row of the ticket's §8 table:
//! - a Thai local number (`0…`), with or without separators, is sent to Identity as `+66…`;
//! - an international number (`+…`) is sent unchanged;
//! - an empty or blank phone is refused `validationFailed` on `phone` with "กรุณากรอกเบอร์โทรศัพท์";
//! - anything else is refused `validationFailed` on `phone` with "เบอร์โทรศัพท์ไม่ถูกต้อง".
//!
//! A refused phone calls Identity never and writes no `users` row (§9): every refusal test checks both.

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;

const EMPTY_PHONE: &str = "กรุณากรอกเบอร์โทรศัพท์";
const INVALID_PHONE: &str = "เบอร์โทรศัพท์ไม่ถูกต้อง";

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// The same owner session as `user_create_test.rs`: an ACTIVE ADMIN + EXECUTIVE bound to `SUB_A`.
async fn owner_session(desk: &Desk, identity: &FakeIdentity, app: &App) -> String {
    let _ = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Owner",
        &[("ADMIN", None), ("EXECUTIVE", None)],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app
        .send(Req::post(
            "/api/auth/login",
            json!({ "identifier": "+66811111111", "password": "pw-owner-01" }),
        ))
        .await;
    reply.session()
}

async fn users_count(desk: &Desk) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&desk.pool)
        .await
        .expect("count users")
}

async fn create_with_phone(app: &App, token: &str, phone: &str) -> Reply {
    app.send(
        Req::post(
            "/api/users",
            json!({
                "name": "Somchai",
                "username": "somchai",
                "phone": phone,
                "password": "temp-password-123",
                "roles": [{ "role": "TELLER", "branch": "MAIN" }]
            }),
        )
        .with_session(token),
    )
    .await
}

/// §8 rows 1–3: the account is created and Identity receives exactly `expected`.
async fn assert_sent_as(typed: &str, expected: &str) {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;
    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = create_with_phone(&app, &token, typed).await;

    assert_eq!(reply.status, 200, "typed {typed:?}: {}", reply.text);
    assert_eq!(reply.json()["status"], "ACTIVE", "typed {typed:?}");
    let sent = identity.received("/auth/register");
    assert_eq!(sent.len(), 1, "typed {typed:?}: one /auth/register");
    assert_eq!(
        sent[0].json()["phone"],
        expected,
        "typed {typed:?}: the phone Identity receives (E.164, §5.3)"
    );

    desk.drop_schema().await;
}

/// §8 rows 4–5 and §9: refused on `phone` with `message`; Identity is not called and no `users` row is written.
async fn assert_refused(typed: &str, message: &str) {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;
    let before = users_count(&desk).await;

    let reply = create_with_phone(&app, &token, typed).await;

    assert_eq!(reply.status, 400, "typed {typed:?}: {}", reply.text);
    let body = reply.json();
    assert_eq!(
        body["error"]["code"], "validationFailed",
        "typed {typed:?}: {body}"
    );
    assert_eq!(body["error"]["field"], "phone", "typed {typed:?}: {body}");
    assert_eq!(body["error"]["message"], message, "typed {typed:?}: {body}");
    assert!(
        identity.received("/auth/register").is_empty(),
        "typed {typed:?}: Identity is not called"
    );
    assert_eq!(
        users_count(&desk).await,
        before,
        "typed {typed:?}: no users row is written"
    );

    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_thai_local_number_is_sent_to_identity_as_plus_66() {
    assert_sent_as("0812341688", "+66812341688").await;
}

#[actix_web::test]
async fn a_thai_local_number_with_dashes_is_sent_as_plus_66() {
    assert_sent_as("081-234-1688", "+66812341688").await;
}

#[actix_web::test]
async fn a_thai_local_number_with_spaces_is_sent_as_plus_66() {
    assert_sent_as("081 234 1688", "+66812341688").await;
}

#[actix_web::test]
async fn a_thai_local_number_with_brackets_is_sent_as_plus_66() {
    assert_sent_as("(081) 234 1688", "+66812341688").await;
}

#[actix_web::test]
async fn an_international_number_is_sent_unchanged() {
    assert_sent_as("+66812341688", "+66812341688").await;
}

#[actix_web::test]
async fn an_empty_phone_is_refused_with_the_empty_message_and_nothing_is_written() {
    assert_refused("", EMPTY_PHONE).await;
}

#[actix_web::test]
async fn a_blank_phone_is_refused_with_the_empty_message_and_nothing_is_written() {
    assert_refused("  ", EMPTY_PHONE).await;
}

#[actix_web::test]
async fn letters_are_refused_as_an_invalid_phone_and_nothing_is_written() {
    assert_refused("abc", INVALID_PHONE).await;
}

#[actix_web::test]
async fn a_number_without_its_leading_zero_is_refused_and_nothing_is_written() {
    assert_refused("812341688", INVALID_PHONE).await;
}

#[actix_web::test]
async fn a_bare_plus_is_refused_and_nothing_is_written() {
    assert_refused("+", INVALID_PHONE).await;
}

#[actix_web::test]
async fn a_bare_zero_is_refused_and_nothing_is_written() {
    assert_refused("0", INVALID_PHONE).await;
}

#[actix_web::test]
async fn login_succeeds_with_a_pool_of_one_connection() {
    use std::str::FromStr;
    use std::time::Duration;

    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let options = PgConnectOptions::from_str(&url)
        .expect("PostgreSQL URL")
        .options([("search_path", desk.schema.as_str())]);
    let single_conn_pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(2))
        .connect_with(options)
        .await
        .expect("connect 1-connection pool to test schema");

    let app = app(state_with(
        &single_conn_pool,
        &identity_env(&identity.url()),
    ))
    .await;
    let token = owner_session(&desk, &identity, &app).await;
    assert!(!token.is_empty());

    single_conn_pool.close().await;
    desk.drop_schema().await;
}
