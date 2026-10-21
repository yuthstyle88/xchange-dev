//! RBAC f20 (F8, #3203): forced-password-change gate in `authenticate_caller`.
//!
//! Behaviour table rows:
//! 1. signed in, flag false, a v3 view the user may read -> 200, as today
//! 2. signed in, flag true, the same view -> 403 `passwordChangeRequired`, no field
//! 3. signed in, flag true, a v3 command -> 403 `passwordChangeRequired`, and nothing written
//! 4. signed in, flag true, a v3 route the user has no permission for -> 403 `passwordChangeRequired` (gate before policy)
//! 5. flag true, `POST /api/auth/session` and `POST /api/auth/logout` -> not gated (200, as today)
//! 6. no session, a Public RPC row -> as today (no gate)
//! 7. an expired session row with the flag -> as today for an expired session (the gate reads only live rows)

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use actix_web::http::{Method, header};
use actix_web::test;
use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::{Value, json};

fn login_req(identifier: &str) -> Req {
    Req::post(
        "/api/auth/login",
        json!({ "identifier": identifier, "password": "pw-from-the-browser-01" }),
    )
}

async fn setup() -> (Desk, FakeIdentity) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    (desk, identity)
}

async fn sign_in_token(desk: &Desk, identity: &FakeIdentity, identifier: &str) -> String {
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let a = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    a.send(login_req(identifier)).await.session()
}

async fn set_must_change(desk: &Desk, token: &str, flag: bool) {
    sqlx::query("UPDATE user_sessions SET must_change_password = $1 WHERE hash = $2")
        .bind(flag)
        .bind(sha256_hex(token))
        .execute(&desk.pool)
        .await
        .expect("update must_change_password");
}

struct V3Reply {
    status: u16,
    text: String,
    body: Value,
}

async fn call_v3(
    desk: &Desk,
    identity: &FakeIdentity,
    method: Method,
    uri: &str,
    session_token: Option<&str>,
    payload: Option<Value>,
) -> V3Reply {
    let state = state_with(&desk.pool, &identity_env(&identity.url()));
    let service = test::init_service(
        actix_web::App::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
    )
    .await;

    let mut req = test::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-real-ip", BROWSER_ADDRESS));

    if let Some(token) = session_token {
        req = req.insert_header((header::COOKIE, format!("{SESSION_COOKIE}={token}")));
    }

    if let Some(body) = payload {
        req = req
            .insert_header((header::ORIGIN, ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header(("idempotency-key", "idem-rbac-f8-0001"))
            .set_payload(body.to_string());
    }

    let resp = test::call_service(&service, req.to_request()).await;
    let status = resp.status().as_u16();
    let bytes = test::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    V3Reply { status, text, body }
}

fn assert_password_change_required(reply: &V3Reply, what: &str) {
    assert_eq!(reply.status, 403, "{what}: {}", reply.text);
    assert_eq!(
        reply.body["error"]["code"],
        json!("passwordChangeRequired"),
        "{what}: {}",
        reply.text
    );
    assert!(
        reply.body["error"].get("field").is_none(),
        "{what}: error.field must be absent: {}",
        reply.text
    );
}

/// Row 1: signed in, flag false, a v3 view the user may read -> 200, as today.
#[actix_web::test]
async fn row1_signed_in_flag_false_v3_view_answers_200() {
    let (desk, identity) = setup().await;
    let _admin = bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin One", &[("ADMIN", None)]).await;
    let token = sign_in_token(&desk, &identity, "admin-one").await;

    // Default flag is false -> 200
    let ok = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/admin_users?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(ok.status, 200, "flag false view: {}", ok.text);
    assert!(
        ok.body["data"].is_array(),
        "flag false view data: {}",
        ok.text
    );

    // Flipping flag true gates the view, and flipping back to false restores 200
    set_must_change(&desk, &token, true).await;
    let gated = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/admin_users?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_password_change_required(&gated, "flagged view");

    set_must_change(&desk, &token, false).await;
    let restored = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/admin_users?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        restored.status, 200,
        "restored flag false view: {}",
        restored.text
    );

    desk.drop_schema().await;
}

/// Row 2: signed in, flag true, the same view -> 403 `passwordChangeRequired`, no field.
#[actix_web::test]
async fn row2_signed_in_flag_true_v3_view_answers_403_password_change_required() {
    let (desk, identity) = setup().await;
    let _admin = bound_user(&desk.pool, SUB_A, "ACTIVE", "Admin Two", &[("ADMIN", None)]).await;
    let token = sign_in_token(&desk, &identity, "admin-two").await;
    set_must_change(&desk, &token, true).await;

    let reply = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/admin_users?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_password_change_required(&reply, "row 2 view with flag true");

    desk.drop_schema().await;
}

/// Row 3: signed in, flag true, a v3 command -> 403 `passwordChangeRequired`, and nothing written.
#[actix_web::test]
async fn row3_signed_in_flag_true_v3_command_answers_403_and_writes_nothing() {
    let (desk, identity) = setup().await;
    let _admin = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Admin Three",
        &[("ADMIN", None)],
    )
    .await;
    let target = bound_user(
        &desk.pool,
        SUB_B,
        "ACTIVE",
        "Teller Target",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let token = sign_in_token(&desk, &identity, "admin-three").await;
    set_must_change(&desk, &token, true).await;

    let reply = call_v3(
        &desk,
        &identity,
        Method::POST,
        "/api/users/disable",
        Some(&token),
        Some(json!({ "userId": target })),
    )
    .await;
    assert_password_change_required(&reply, "row 3 command with flag true");

    let (status, version): (String, i32) =
        sqlx::query_as("SELECT status, version FROM users WHERE id = $1::uuid")
            .bind(&target)
            .fetch_one(&desk.pool)
            .await
            .expect("query target user");
    assert_eq!(status, "ACTIVE", "target user status must not change");
    assert_eq!(version, 1, "target user version must not change");

    desk.drop_schema().await;
}

/// Row 4: signed in, flag true, a v3 route the user has no permission for -> 403 `passwordChangeRequired` (gate before policy).
#[actix_web::test]
async fn row4_signed_in_flag_true_without_permission_answers_403_password_change_required() {
    let (desk, identity) = setup().await;
    let _teller = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Teller Four",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let token = sign_in_token(&desk, &identity, "teller-four").await;
    set_must_change(&desk, &token, true).await;

    // TELLER lacks UsersManage for admin_users, but gate runs before Policy::authorize
    let reply = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/admin_users?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_password_change_required(&reply, "row 4 gate before policy");

    desk.drop_schema().await;
}

/// Row 5: flag true, `POST /api/auth/session` and `POST /api/auth/logout` -> not gated (200, as today).
#[actix_web::test]
async fn row5_flag_true_auth_session_and_logout_are_not_gated() {
    let (desk, identity) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Teller Five",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let token = sign_in_token(&desk, &identity, "teller-five").await;
    set_must_change(&desk, &token, true).await;

    // Verify v3 view is gated on this session
    let gated = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/me?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_password_change_required(&gated, "v3 view on flagged session");

    // POST /api/auth/session is NOT gated
    let a = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let sess_reply = a
        .send(Req::post("/api/auth/session", json!({})).with_session(&token))
        .await;
    assert_eq!(sess_reply.status, 200, "auth/session: {}", sess_reply.text);
    assert_eq!(sess_reply.json()["user"]["id"], json!(user));

    // POST /api/auth/logout is NOT gated
    let logout_reply = a
        .send(Req::post("/api/auth/logout", json!({})).with_session(&token))
        .await;
    assert_eq!(
        logout_reply.status, 200,
        "auth/logout: {}",
        logout_reply.text
    );
    assert_eq!(all_sessions(&desk.pool).await, 0);

    desk.drop_schema().await;
}

/// Row 6: no session, a Public RPC row -> as today (no gate).
#[actix_web::test]
async fn row6_no_session_public_rpc_row_is_not_gated() {
    let (desk, identity) = setup().await;
    let _teller = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Teller Six",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let token = sign_in_token(&desk, &identity, "teller-six").await;
    set_must_change(&desk, &token, true).await;

    // With no session, Public RPC view booking_slots answers 200
    let anon = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/booking_slots?branch=MAIN",
        None,
        None,
    )
    .await;
    assert_eq!(
        anon.status, 200,
        "public rpc without session: {}",
        anon.text
    );
    assert!(anon.body["data"]["slots"].is_array());

    // Contrast: with the flagged session, even the Public RPC view is gated
    let gated = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/booking_slots?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_password_change_required(&gated, "public rpc with flagged session");

    desk.drop_schema().await;
}

/// Row 7: an expired session row with the flag -> as today for an expired session (the gate reads only live rows).
#[actix_web::test]
async fn row7_expired_session_row_with_flag_behaves_as_expired_session() {
    let (desk, identity) = setup().await;
    let _admin = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Admin Seven",
        &[("ADMIN", None)],
    )
    .await;
    let token = sign_in_token(&desk, &identity, "admin-seven").await;
    set_must_change(&desk, &token, true).await;

    // While live, the session is gated with 403 passwordChangeRequired
    let live = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/admin_users?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_password_change_required(&live, "live flagged session");

    // Expire the session row while keeping must_change_password = true
    sqlx::query("UPDATE user_sessions SET expires = now() - interval '10 seconds' WHERE hash = $1")
        .bind(sha256_hex(&token))
        .execute(&desk.pool)
        .await
        .expect("expire session");

    // Protected route answers 401 signIn (not 403 passwordChangeRequired)
    let expired_protected = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/admin_users?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        expired_protected.status, 401,
        "expired session on protected view: {}",
        expired_protected.text
    );
    assert_eq!(expired_protected.body["error"]["code"], json!("signIn"));

    // Public RPC route answers 200 (not 403 passwordChangeRequired)
    let expired_public = call_v3(
        &desk,
        &identity,
        Method::GET,
        "/api/v3/views/booking_slots?branch=MAIN",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        expired_public.status, 200,
        "expired session on public view: {}",
        expired_public.text
    );

    desk.drop_schema().await;
}
