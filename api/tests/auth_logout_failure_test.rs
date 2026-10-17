//! #2647: When `DELETE FROM user_sessions` fails during `POST /api/auth/logout`, the route must
//! answer 503 `identityUnavailable`, keep the `__Host-xc_session` cookie, and leave the
//! `user_sessions` row intact instead of reporting `{"saved": true}` and clearing the cookie.

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;

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

/// Row 1: signed in; `DELETE FROM user_sessions` raises -> 503 `identityUnavailable`,
/// `__Host-xc_session` not cleared, and the session row still exists.
#[actix_web::test]
async fn logout_when_session_delete_fails_answers_503_keeps_cookie_and_keeps_session_row() {
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
    assert_eq!(sessions_of(&desk.pool, &user).await, 1);

    sqlx::query(
        "CREATE FUNCTION fail_user_sessions_delete() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
             RAISE EXCEPTION 'simulated user_sessions delete failure'; \
         END; \
         $$",
    )
    .execute(&desk.pool)
    .await
    .expect("create trigger function");

    sqlx::query(
        "CREATE TRIGGER tr_fail_user_sessions_delete \
         BEFORE DELETE ON user_sessions \
         FOR EACH ROW EXECUTE FUNCTION fail_user_sessions_delete()",
    )
    .execute(&desk.pool)
    .await
    .expect("create trigger on user_sessions");

    let reply = app
        .send(Req::post("/api/auth/logout", json!({})).with_session(&token))
        .await;

    assert_eq!(reply.status, 503, "{}", reply.text);
    assert_eq!(
        reply.refusal(),
        ("identityUnavailable".to_owned(), UNAVAILABLE.to_owned())
    );
    assert!(
        !reply.cookies.contains_key(SESSION_COOKIE),
        "503 must not clear {SESSION_COOKIE}: {:?}",
        reply.cookies
    );
    assert!(
        !reply.cookies.contains_key(OWNER_COOKIE) && !reply.cookies.contains_key(STAFF_COOKIE),
        "503 must not attach logout cookies: {:?}",
        reply.cookies
    );
    assert_eq!(
        sessions_of(&desk.pool, &user).await,
        1,
        "the user_sessions row must still exist when delete fails"
    );

    desk.drop_schema().await;
}

/// Row 2: signed in; delete succeeds -> 200 `{"saved":true}`, cookies cleared, row gone.
#[actix_web::test]
async fn logout_when_session_delete_succeeds_answers_200_clears_cookies_and_deletes_row() {
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

    let reply = app
        .send(Req::post("/api/auth/logout", json!({})).with_session(&token))
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json(), json!({ "saved": true }));
    for name in [SESSION_COOKIE, OWNER_COOKIE, STAFF_COOKIE] {
        let c = reply
            .cookies
            .get(name)
            .unwrap_or_else(|| panic!("{name} is cleared: {:?}", reply.cookies));
        assert_eq!((c.value.as_str(), c.max_age), ("", 0), "{name}");
    }
    assert_eq!(sessions_of(&desk.pool, &user).await, 0);

    desk.drop_schema().await;
}

/// Row 3: no session cookie -> 200 `{"saved":true}`, cookies cleared.
#[actix_web::test]
async fn logout_without_session_cookie_answers_200_and_clears_cookies() {
    let (desk, _identity, app) = setup().await;

    let reply = app.send(Req::post("/api/auth/logout", json!({}))).await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json(), json!({ "saved": true }));
    for name in [SESSION_COOKIE, OWNER_COOKIE, STAFF_COOKIE] {
        let c = reply
            .cookies
            .get(name)
            .unwrap_or_else(|| panic!("{name} is cleared: {:?}", reply.cookies));
        assert_eq!((c.value.as_str(), c.max_age), ("", 0), "{name}");
    }

    desk.drop_schema().await;
}

/// Row 4: an unknown or expired token -> 200 `{"saved":true}`: the delete finds nothing (or deletes
/// the expired row) and is `Ok`.
#[actix_web::test]
async fn logout_with_unknown_or_expired_token_answers_200_and_clears_cookies() {
    let (desk, identity, app) = setup().await;

    // Unknown token
    let unknown_reply = app
        .send(Req::post("/api/auth/logout", json!({})).with_session(&"ab".repeat(32)))
        .await;
    assert_eq!(unknown_reply.status, 200, "{}", unknown_reply.text);
    assert_eq!(unknown_reply.json(), json!({ "saved": true }));
    for name in [SESSION_COOKIE, OWNER_COOKIE, STAFF_COOKIE] {
        let c = unknown_reply
            .cookies
            .get(name)
            .unwrap_or_else(|| panic!("{name} is cleared: {:?}", unknown_reply.cookies));
        assert_eq!((c.value.as_str(), c.max_age), ("", 0), "{name}");
    }

    // Expired token
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

    sqlx::query("UPDATE user_sessions SET expires = now() - interval '1 hour'")
        .execute(&desk.pool)
        .await
        .expect("expire session row");

    let expired_reply = app
        .send(Req::post("/api/auth/logout", json!({})).with_session(&token))
        .await;
    assert_eq!(expired_reply.status, 200, "{}", expired_reply.text);
    assert_eq!(expired_reply.json(), json!({ "saved": true }));
    assert_eq!(sessions_of(&desk.pool, &user).await, 0);

    desk.drop_schema().await;
}

/// Row 5: Identity's `/auth/logout` fails after a successful delete -> 200 `{"saved":true}` (best effort).
#[actix_web::test]
async fn logout_when_identity_logout_fails_after_session_delete_still_answers_200() {
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
    identity.answer("/auth/logout", 503, r#"{"error":"unavailable"}"#);
    let token = app.send(login("nok")).await.session();

    let reply = app
        .send(Req::post("/api/auth/logout", json!({})).with_session(&token))
        .await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json(), json!({ "saved": true }));
    assert_eq!(sessions_of(&desk.pool, &user).await, 0);

    desk.drop_schema().await;
}
