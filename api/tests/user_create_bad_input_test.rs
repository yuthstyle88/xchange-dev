//! Ticket #2616 — userCreate answers 500 for a non-uuid userId or a role branch that does not exist.
//!
//! Behaviour table rows 1–5:
//! 1: userId: "abc" (non-uuid) -> 400 validationFailed field userId; Identity not called; nothing written.
//! 2: roles:[{"role":"TELLER","branch":"NOPE"}], NOPE not in branches -> 400 validationFailed field roles; Identity not called; nothing written.
//! 3: userId a well-formed uuid of no user -> 400 validationFailed field userId (kept).
//! 4: roles:[{"role":"TELLER","branch":"MAIN"}], MAIN exists -> 200, as today (kept).
//! 5: branch:"bad branch!" (malformed) -> 400 validationFailed field roles "สาขาไม่ถูกต้อง" (kept).

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

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

async fn user_roles_count(desk: &Desk) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM user_roles")
        .fetch_one(&desk.pool)
        .await
        .expect("count user_roles")
}

/// Row 1: userId is not a uuid (e.g. "abc") -> 400 validationFailed field userId;
/// Identity not called; nothing written.
#[actix_web::test]
async fn test_row_1_user_id_not_uuid_refused_400_validation_failed() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;
    let before_users = users_count(&desk).await;
    let before_roles = user_roles_count(&desk).await;

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "BadUserIdStaff",
                    "phone": "+66812345671",
                    "password": "temp-password-1",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }],
                    "userId": "abc"
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(
        reply.status, 400,
        "status: {}, body: {}",
        reply.status, reply.text
    );
    let body = reply.json();
    assert_eq!(body["error"]["code"], "validationFailed");
    assert_eq!(body["error"]["field"], "userId");
    assert_eq!(body["error"]["message"], "ไม่พบผู้ใช้งาน");
    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity received no /auth/register"
    );
    assert_eq!(
        users_count(&desk).await,
        before_users,
        "no users row was written"
    );
    assert_eq!(
        user_roles_count(&desk).await,
        before_roles,
        "no user_roles row was written"
    );

    desk.drop_schema().await;
}

/// Row 2: roles:[{"role":"TELLER","branch":"NOPE"}], NOPE not in branches ->
/// 400 validationFailed field roles "สาขาไม่ถูกต้อง"; Identity not called; nothing written.
#[actix_web::test]
async fn test_row_2_unknown_branch_refused_400_validation_failed() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;
    let before_users = users_count(&desk).await;
    let before_roles = user_roles_count(&desk).await;

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "BadBranchStaff",
                    "phone": "+66812345672",
                    "password": "temp-password-2",
                    "roles": [{ "role": "TELLER", "branch": "NOPE" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(
        reply.status, 400,
        "status: {}, body: {}",
        reply.status, reply.text
    );
    let body = reply.json();
    assert_eq!(body["error"]["code"], "validationFailed");
    assert_eq!(body["error"]["field"], "roles");
    assert_eq!(body["error"]["message"], "สาขาไม่ถูกต้อง");
    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity received no /auth/register"
    );
    assert_eq!(
        users_count(&desk).await,
        before_users,
        "no users row was written"
    );
    assert_eq!(
        user_roles_count(&desk).await,
        before_roles,
        "no user_roles row was written"
    );

    desk.drop_schema().await;
}

/// Row 3: userId a well-formed uuid of no user ->
/// 400 validationFailed field userId (kept, user_service.rs:277-282).
#[actix_web::test]
async fn test_row_3_well_formed_uuid_no_user_refused_400_validation_failed() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;
    let before_users = users_count(&desk).await;
    let before_roles = user_roles_count(&desk).await;

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "UnknownUuidStaff",
                    "phone": "+66812345673",
                    "password": "temp-password-3",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }],
                    "userId": "00000000-0000-0000-0000-999999999999"
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(
        reply.status, 400,
        "status: {}, body: {}",
        reply.status, reply.text
    );
    let body = reply.json();
    assert_eq!(body["error"]["code"], "validationFailed");
    assert_eq!(body["error"]["field"], "userId");
    assert_eq!(body["error"]["message"], "ไม่พบผู้ใช้งาน");
    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity received no /auth/register"
    );
    assert_eq!(
        users_count(&desk).await,
        before_users,
        "no users row was written"
    );
    assert_eq!(
        user_roles_count(&desk).await,
        before_roles,
        "no user_roles row was written"
    );

    desk.drop_schema().await;
}

/// Row 4: roles:[{"role":"TELLER","branch":"MAIN"}], MAIN exists -> 200, as today (kept).
#[actix_web::test]
async fn test_row_4_valid_branch_main_answers_200() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;

    identity.answer(
        "/auth/register",
        201,
        &json!({ "identityId": SUB_B }).to_string(),
    );

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "ValidStaff",
                    "phone": "+66812345674",
                    "password": "temp-password-4",
                    "roles": [{ "role": "TELLER", "branch": "MAIN" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(
        reply.status, 200,
        "status: {}, body: {}",
        reply.status, reply.text
    );
    let body = reply.json();
    assert_eq!(body["status"], "ACTIVE");
    assert_eq!(identity.received("/auth/register").len(), 1);

    desk.drop_schema().await;
}

/// Row 5: branch:"bad branch!" (malformed) -> 400 validationFailed field roles "สาขาไม่ถูกต้อง" (kept, users.rs:122).
#[actix_web::test]
async fn test_row_5_malformed_branch_refused_400_validation_failed() {
    let (desk, identity, app) = setup().await;
    let token = owner_session(&desk, &identity, &app).await;
    let before_users = users_count(&desk).await;
    let before_roles = user_roles_count(&desk).await;

    let reply = app
        .send(
            Req::post(
                "/api/users",
                json!({
                    "name": "MalformedBranchStaff",
                    "phone": "+66812345675",
                    "password": "temp-password-5",
                    "roles": [{ "role": "TELLER", "branch": "bad branch!" }]
                }),
            )
            .with_session(&token),
        )
        .await;

    assert_eq!(
        reply.status, 400,
        "status: {}, body: {}",
        reply.status, reply.text
    );
    let body = reply.json();
    assert_eq!(body["error"]["code"], "validationFailed");
    assert_eq!(body["error"]["field"], "roles");
    assert_eq!(body["error"]["message"], "สาขาไม่ถูกต้อง");
    assert!(
        identity.received("/auth/register").is_empty(),
        "Identity received no /auth/register"
    );
    assert_eq!(
        users_count(&desk).await,
        before_users,
        "no users row was written"
    );
    assert_eq!(
        user_roles_count(&desk).await,
        before_roles,
        "no user_roles row was written"
    );

    desk.drop_schema().await;
}
