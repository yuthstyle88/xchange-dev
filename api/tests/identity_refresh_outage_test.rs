//! #2206: An Identity outage (5xx or unreachable) during `/api/auth/refresh` returns 503
//! `identityUnavailable`, keeps the cookie, and keeps the `user_sessions` row so the session
//! remains usable once Identity recovers. Definite refusals (e.g. 401 `refresh_token_reuse`,
//! `invalid_refresh_token`) still delete the `user_sessions` row and clear the cookie.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";

fn login(identifier: &str) -> Req {
    Req::post(
        "/api/auth/login",
        json!({ "identifier": identifier, "password": "pw-from-the-browser-01" }),
    )
}

/// A fresh schema, a fake Identity, and the app wired to it.
async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// Row 1: Identity answers 503 `{}` -> 503 `identityUnavailable`, cookie kept, `user_sessions` row kept (1).
#[actix_web::test]
async fn refresh_when_identity_answers_503_keeps_the_session_row_and_cookie() {
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
    let token = app.send(login("nok")).await.session();

    identity.answer("/auth/refresh", 503, "{}");
    let reply = app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&token))
        .await;
    assert_eq!(reply.status, 503, "{}", reply.text);
    assert_eq!(
        reply.refusal(),
        ("identityUnavailable".to_owned(), UNAVAILABLE.to_owned())
    );
    assert!(
        !reply.cookies.contains_key(SESSION_COOKIE),
        "503 keeps the existing session cookie untouched: {:?}",
        reply.cookies
    );
    assert_eq!(
        sessions_of(&desk.pool, &user).await,
        1,
        "the user_sessions row survives a 503 outage"
    );
    desk.drop_schema().await;
}

/// Row 2: Identity answers 500 or 502 -> 503 `identityUnavailable`, `user_sessions` row kept (1).
#[actix_web::test]
async fn refresh_when_identity_answers_500_or_502_keeps_the_session_row() {
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
    let token = app.send(login("nok")).await.session();

    for status in [500, 502] {
        identity.answer("/auth/refresh", status, r#"{"error":"internal"}"#);
        let reply = app
            .send(Req::post("/api/auth/refresh", json!({})).with_session(&token))
            .await;
        assert_eq!(reply.status, 503, "status {status}: {}", reply.text);
        assert_eq!(
            reply.refusal(),
            ("identityUnavailable".to_owned(), UNAVAILABLE.to_owned())
        );
        assert_eq!(
            sessions_of(&desk.pool, &user).await,
            1,
            "the user_sessions row survives Identity HTTP {status}"
        );
    }
    desk.drop_schema().await;
}

/// Row 3: Connection refused (unreachable Identity URL) -> 503 `identityUnavailable`, `user_sessions` row kept (1).
#[actix_web::test]
async fn refresh_when_identity_connection_is_refused_keeps_the_session_row() {
    let (desk, identity, app_online) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let token = app_online.send(login("nok")).await.session();

    let dead_url = format!("http://127.0.0.1:{}", closed_port());
    let app_offline = app(state_with(&desk.pool, &identity_env(&dead_url))).await;

    let reply = app_offline
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&token))
        .await;
    assert_eq!(reply.status, 503, "{}", reply.text);
    assert_eq!(
        reply.refusal(),
        ("identityUnavailable".to_owned(), UNAVAILABLE.to_owned())
    );
    assert_eq!(
        sessions_of(&desk.pool, &user).await,
        1,
        "the user_sessions row survives a connection refusal"
    );
    desk.drop_schema().await;
}

/// Row 4: Identity answers 401 `{"error":"refresh_token_reuse"}` -> 401 `signIn`, cookie cleared, 0 rows.
#[actix_web::test]
async fn refresh_refused_with_refresh_token_reuse_deletes_the_session_row_and_clears_cookie() {
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
    let token = app.send(login("nok")).await.session();

    identity.answer("/auth/refresh", 401, r#"{"error":"refresh_token_reuse"}"#);
    let reply = app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&token))
        .await;
    assert_eq!(reply.status, 401, "{}", reply.text);
    assert_eq!(reply.refusal(), ("signIn".to_owned(), SIGN_IN.to_owned()));
    let cleared = &reply.cookies[SESSION_COOKIE];
    assert_eq!((cleared.value.as_str(), cleared.max_age), ("", 0));
    assert_eq!(sessions_of(&desk.pool, &user).await, 0);
    desk.drop_schema().await;
}

/// Row 5: Identity answers 401 `{"error":"invalid_refresh_token"}` -> 401 `signIn`, cookie cleared, 0 rows.
#[actix_web::test]
async fn refresh_refused_with_invalid_refresh_token_deletes_the_session_row_and_clears_cookie() {
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
    let token = app.send(login("nok")).await.session();

    identity.answer("/auth/refresh", 401, r#"{"error":"invalid_refresh_token"}"#);
    let reply = app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&token))
        .await;
    assert_eq!(reply.status, 401, "{}", reply.text);
    assert_eq!(reply.refusal(), ("signIn".to_owned(), SIGN_IN.to_owned()));
    let cleared = &reply.cookies[SESSION_COOKIE];
    assert_eq!((cleared.value.as_str(), cleared.max_age), ("", 0));
    assert_eq!(sessions_of(&desk.pool, &user).await, 0);
    desk.drop_schema().await;
}

/// Row 6: 503, then Identity recovers and answers 200 with `VALID_A` + `REFRESH_2` ->
/// second refresh 200 `{user}`, 1 session row, refresh token replaced.
#[actix_web::test]
async fn refresh_after_503_recovery_succeeds_and_replaces_the_refresh_token() {
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
    identity.answer("/auth/logout", 204, "");
    let token = app.send(login("nok")).await.session();

    identity.answer("/auth/refresh", 503, "{}");
    let first = app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&token))
        .await;
    assert_eq!(first.status, 503, "{}", first.text);
    assert_eq!(sessions_of(&desk.pool, &user).await, 1);

    identity.login_answers("/auth/refresh", VALID_A, REFRESH_2);
    let second = app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&token))
        .await;
    assert_eq!(second.status, 200, "{}", second.text);
    assert_eq!(second.json()["user"]["id"], json!(user));
    assert_eq!(sessions_of(&desk.pool, &user).await, 1);

    app.send(Req::post("/api/auth/logout", json!({})).with_session(&token))
        .await;
    assert_eq!(
        identity.received("/auth/logout")[0].json(),
        json!({ "refresh_token": REFRESH_2 }),
        "the stored refresh token was replaced on the recovered refresh"
    );
    desk.drop_schema().await;
}
