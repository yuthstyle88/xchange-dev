//! RBAC F11b (#3206) tests for `POST /api/auth/password`.

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::{Value, json};

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const INVALID_CREDENTIALS: &str = "ชื่อผู้ใช้หรือรหัสผ่านไม่ถูกต้อง";

fn login(identifier: &str) -> Req {
    Req::post(
        "/api/auth/login",
        json!({ "identifier": identifier, "password": "pw-from-the-browser-01" }),
    )
}

fn session_of(token: &str) -> Req {
    Req::post("/api/auth/session", json!({})).with_session(token)
}

fn password_change(token: &str, body: Value) -> Req {
    Req::post("/api/auth/password", body).with_session(token)
}

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// Row 1: Right current password, valid new one -> 200 `{"saved":true}`; the response clears the
/// session cookie; a following `POST /api/auth/session` with the old cookie is signed out.
#[actix_web::test]
async fn right_current_password_and_valid_new_one_clears_cookies_and_ends_session() {
    let (desk, identity, app) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    identity.login_answers("/auth/refresh", VALID_A, REFRESH_2);
    identity.answer(
        "/auth/password/change",
        200,
        &json!({ "changed": true }).to_string(),
    );

    let token = app.send(login("nok")).await.session();

    let reply = app
        .send(password_change(
            &token,
            json!({
                "currentPassword": "pw-from-the-browser-01",
                "newPassword": "NewStrongPass#2026",
            }),
        ))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json(), json!({ "saved": true }));

    // OR-7.m w2 F-b (#1027 issuecomment-6305469252): only the session cookie is cleared; the v2
    // cookies `__Host-xc_owner` / `__Host-xc_staff` are no longer sent at all.
    let c = reply
        .cookies
        .get(SESSION_COOKIE)
        .unwrap_or_else(|| panic!("{SESSION_COOKIE} is cleared: {:?}", reply.cookies));
    assert_eq!((c.value.as_str(), c.max_age), ("", 0), "{SESSION_COOKIE}");
    for old in [OWNER_COOKIE, STAFF_COOKIE] {
        assert!(
            !reply.cookies.contains_key(old),
            "{old} is not sent: {:?}",
            reply.cookies
        );
    }

    assert_eq!(sessions_of(&desk.pool, &user).await, 0);
    assert_eq!(
        app.send(session_of(&token)).await.json(),
        json!({ "user": null }),
        "a following POST /api/auth/session with the old cookie is signed out"
    );

    let sent = identity.received("/auth/password/change");
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].json(),
        json!({
            "currentPassword": "pw-from-the-browser-01",
            "newPassword": "NewStrongPass#2026",
        })
    );
    assert_eq!(
        sent[0].header("authorization"),
        Some(&*format!("Bearer {VALID_A}"))
    );

    desk.drop_schema().await;
}

/// Row 2: Wrong current password (fake Identity refuses) -> 401 `invalidCredentials`; the session still works.
#[actix_web::test]
async fn wrong_current_password_returns_401_invalid_credentials_and_keeps_session() {
    let (desk, identity, app) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    identity.login_answers("/auth/refresh", VALID_A, REFRESH_2);
    identity.answer(
        "/auth/password/change",
        401,
        &json!({ "error": "invalid_credentials" }).to_string(),
    );

    let token = app.send(login("nok")).await.session();

    let reply = app
        .send(password_change(
            &token,
            json!({
                "currentPassword": "wrong-current-password",
                "newPassword": "NewStrongPass#2026",
            }),
        ))
        .await;
    assert_eq!(reply.status, 401, "{}", reply.text);
    assert_eq!(
        reply.refusal(),
        (
            "invalidCredentials".to_owned(),
            INVALID_CREDENTIALS.to_owned()
        )
    );
    assert!(reply.sets_no_session());

    let check = app.send(session_of(&token)).await;
    assert_eq!(check.status, 200);
    assert_eq!(
        check.json()["user"]["id"],
        json!(user),
        "the session still works after wrong current password"
    );

    desk.drop_schema().await;
}

/// Row 3: New password refused by policy -> 400 `validationFailed`, field `newPassword`.
#[actix_web::test]
async fn new_password_refused_by_policy_returns_400_validation_failed_on_new_password() {
    let (desk, identity, app) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    identity.login_answers("/auth/refresh", VALID_A, REFRESH_2);
    identity.answer(
        "/auth/password/change",
        422,
        &json!({ "error": "password_policy_violation" }).to_string(),
    );

    let token = app.send(login("nok")).await.session();

    let reply = app
        .send(password_change(
            &token,
            json!({
                "currentPassword": "pw-from-the-browser-01",
                "newPassword": "weak",
            }),
        ))
        .await;
    assert_eq!(reply.status, 400, "{}", reply.text);
    let body = reply.json();
    assert_eq!(body["error"]["code"], json!("validationFailed"));
    assert_eq!(body["error"]["field"], json!("newPassword"));
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty())
    );

    let check = app.send(session_of(&token)).await;
    assert_eq!(check.status, 200);
    assert_eq!(
        check.json()["user"]["id"],
        json!(user),
        "the session still works after policy refusal"
    );

    desk.drop_schema().await;
}

/// Row 4: Missing `newPassword` -> 400 `validationFailed`, field `newPassword`; Identity never called.
#[actix_web::test]
async fn missing_new_password_returns_400_validation_failed_and_never_calls_identity() {
    let (desk, identity, app) = setup().await;
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);

    let token = app.send(login("nok")).await.session();

    for payload in [
        json!({ "currentPassword": "pw-from-the-browser-01" }),
        json!({ "currentPassword": "pw-from-the-browser-01", "newPassword": "" }),
    ] {
        let reply = app.send(password_change(&token, payload)).await;
        assert_eq!(reply.status, 400, "{}", reply.text);
        let body = reply.json();
        assert_eq!(body["error"]["code"], json!("validationFailed"));
        assert_eq!(body["error"]["field"], json!("newPassword"));
    }

    let missing_current = app
        .send(password_change(
            &token,
            json!({ "newPassword": "NewStrongPass#2026" }),
        ))
        .await;
    assert_eq!(missing_current.status, 400, "{}", missing_current.text);
    let current_body = missing_current.json();
    assert_eq!(current_body["error"]["code"], json!("validationFailed"));
    assert_eq!(current_body["error"]["field"], json!("currentPassword"));

    assert!(
        identity.received("/auth/refresh").is_empty(),
        "Identity refresh must not be called on missing fields"
    );
    assert!(
        identity.received("/auth/password/change").is_empty(),
        "Identity password change must not be called on missing fields"
    );

    desk.drop_schema().await;
}

/// Row 5: No session cookie -> 401 `signIn`.
#[actix_web::test]
async fn no_session_cookie_returns_401_sign_in() {
    let (desk, identity, app) = setup().await;

    let reply = app
        .send(Req::post(
            "/api/auth/password",
            json!({
                "currentPassword": "pw-from-the-browser-01",
                "newPassword": "NewStrongPass#2026",
            }),
        ))
        .await;
    assert_eq!(reply.status, 401, "{}", reply.text);
    assert_eq!(reply.refusal(), ("signIn".to_owned(), SIGN_IN.to_owned()));
    assert!(
        identity.calls().is_empty(),
        "Identity must not be called when there is no session cookie"
    );

    desk.drop_schema().await;
}

/// Row 6: A flagged session (`must_change_password = true`) -> allowed (this route is the way out of the F8 gate).
#[actix_web::test]
async fn a_flagged_session_is_allowed_to_change_password() {
    let (desk, identity, app) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    let mut flagged_login = login_body(VALID_A, REFRESH_1);
    flagged_login["mustChangePassword"] = json!(true);
    identity.answer("/auth/login", 200, &flagged_login.to_string());
    identity.login_answers("/auth/refresh", VALID_A, REFRESH_2);
    identity.answer(
        "/auth/password/change",
        200,
        &json!({ "changed": true }).to_string(),
    );

    let token = app.send(login("nok")).await.session();

    let stored_flag: bool =
        sqlx::query_scalar("SELECT must_change_password FROM user_sessions WHERE hash = $1")
            .bind(sha256_hex(&token))
            .fetch_one(&desk.pool)
            .await
            .expect("session row exists");
    assert!(
        stored_flag,
        "session must be flagged with must_change_password = true"
    );

    let reply = app
        .send(password_change(
            &token,
            json!({
                "currentPassword": "pw-from-the-browser-01",
                "newPassword": "NewStrongPass#2026",
            }),
        ))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json(), json!({ "saved": true }));
    assert_eq!(sessions_of(&desk.pool, &user).await, 0);

    desk.drop_schema().await;
}
