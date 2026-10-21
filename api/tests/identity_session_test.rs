//! Implementer extra tests for OR-3 (§9 Edge Cases & lane controller rulings).
//!
//! Covers:
//! 1. An expired session row -> nobody (`{"user": null}`).
//! 2. HKDF derived key: derived key is 32 bytes and not the raw secret; encrypting under
//!    one secret cannot be decrypted under another rotated secret, ending the session.
//! 3. Refresh whose new token names a different subject -> session ends and is deleted.
//! 4. Missing IDENTITY_API_KEY is handled and logged/warned.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;
use xc_auth::application::session::{
    decrypt_refresh_token, derive_refresh_token_key, encrypt_refresh_token,
};

fn login_req(identifier: &str) -> Req {
    Req::post(
        "/api/auth/login",
        json!({ "identifier": identifier, "password": "pw-from-the-browser-01" }),
    )
}

fn session_req(token: &str) -> Req {
    Req::post("/api/auth/session", json!({})).with_session(token)
}

fn refresh_req(token: &str) -> Req {
    Req::post("/api/auth/refresh", json!({})).with_session(token)
}

async fn setup() -> (Desk, FakeIdentity, App) {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    (desk, identity, app)
}

/// §9: An expired session row -> nobody.
#[actix_web::test]
async fn expired_session_row_reads_nobody() {
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

    let token = app.send(login_req("somchai")).await.session();

    // Verify session works initially
    let reply = app.send(session_req(&token)).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["user"]["id"], json!(user));

    // Expire the session in database
    sqlx::query("UPDATE user_sessions SET expires = now() - interval '10 seconds' WHERE hash = $1")
        .bind(sha256_hex(&token))
        .execute(&desk.pool)
        .await
        .expect("expire session");

    // After expiration, reading session returns null user
    let after = app.send(session_req(&token)).await;
    assert_eq!(after.status, 200);
    assert_eq!(after.json(), json!({ "user": null }));

    desk.drop_schema().await;
}

/// §9 & ruling: HKDF derived key test and secret rotation behavior.
#[actix_web::test]
async fn hkdf_key_derivation_and_secret_rotation() {
    // Known-answer test (KAT) per RFC 5869 for info "xc-v3 refresh-token v1".
    // Computed independently via:
    // python3 -c 'import hmac, hashlib; secret = b"my-fixed-secret-kat-value-12345678"; info = b"xc-v3 refresh-token v1"; prk = hmac.new(b"\x00" * 32, secret, hashlib.sha256).digest(); print(hmac.new(prk, info + b"\x01", hashlib.sha256).hexdigest())'
    const EXPECTED_KAT_HEX: &str =
        "e6d85be39d0541170d48d45584267b68d8cb5dcdbed9440bff9c647ef61523c5";
    let kat_secret = "my-fixed-secret-kat-value-12345678";
    let kat_derived = derive_refresh_token_key(kat_secret);
    let kat_hex: String = kat_derived.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        kat_hex, EXPECTED_KAT_HEX,
        "derived refresh-token key must match independently computed RFC 5869 HKDF-SHA256 KAT with info 'xc-v3 refresh-token v1'"
    );

    let secret_1 = "client-secret-version-1-at-least-32-bytes-long";
    let secret_2 = "client-secret-version-2-after-rotation-key";

    let key_1 = derive_refresh_token_key(secret_1);
    let key_2 = derive_refresh_token_key(secret_2);

    // Derived key must not be the raw secret
    assert_ne!(key_1.as_slice(), secret_1.as_bytes());
    assert_ne!(key_1, key_2);

    // Encryption under secret 1 cannot be decrypted under secret 2
    let plain = "my-secret-refresh-token-12345";
    let encrypted = encrypt_refresh_token(secret_1, plain).expect("encrypt");
    let decrypted = decrypt_refresh_token(secret_1, &encrypted).expect("decrypt with same secret");
    assert_eq!(decrypted, plain);

    assert!(
        decrypt_refresh_token(secret_2, &encrypted).is_err(),
        "decrypting with different secret must fail"
    );

    // End-to-end app test: rotating client secret makes refresh fail and ends session
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();

    // App with secret 1
    let mut env1 = identity_env(&identity.url());
    if let Some(entry) = env1
        .iter_mut()
        .find(|(k, _)| *k == "IDENTITY_CLIENT_SECRET")
    {
        entry.1 = secret_1.to_string();
    }
    let app1 = app(state_with(&desk.pool, &env1)).await;

    let _user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Somsak",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);

    let token = app1.send(login_req("somsak")).await.session();
    assert_eq!(all_sessions(&desk.pool).await, 1);

    // Rebuild app with secret 2 (rotated secret)
    let mut env2 = identity_env(&identity.url());
    if let Some(entry) = env2
        .iter_mut()
        .find(|(k, _)| *k == "IDENTITY_CLIENT_SECRET")
    {
        entry.1 = secret_2.to_string();
    }
    let app2 = app(state_with(&desk.pool, &env2)).await;

    // Refresh attempt fails because refresh token cannot be decrypted
    let refresh_reply = app2.send(refresh_req(&token)).await;
    assert_eq!(refresh_reply.status, 401);
    assert_eq!(refresh_reply.refusal().0, "signIn");

    // Session row is removed
    assert_eq!(all_sessions(&desk.pool).await, 0);

    desk.drop_schema().await;
}

/// §9: A refresh whose new token names another subject -> session ends and row deleted.
#[actix_web::test]
async fn refresh_with_different_subject_ends_session() {
    let (desk, identity, app) = setup().await;
    let _user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Somying",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);

    let token = app.send(login_req("somying")).await.session();
    assert_eq!(all_sessions(&desk.pool).await, 1);

    // When refreshing, identity responds with tokens for SUB_B instead of SUB_A
    identity.login_answers("/auth/refresh", VALID_B, REFRESH_2);

    let reply = app.send(refresh_req(&token)).await;
    assert_eq!(reply.status, 401);
    assert_eq!(reply.refusal().0, "signIn");

    // The session row was deleted
    assert_eq!(all_sessions(&desk.pool).await, 0);

    desk.drop_schema().await;
}

/// §9 & ruling: IDENTITY_API_KEY is optional, and settings load correctly without it.
#[actix_web::test]
async fn missing_identity_api_key_is_optional_and_usable() {
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();

    let mut env = identity_env(&identity.url());
    env.retain(|(k, _)| *k != "IDENTITY_API_KEY");

    let state = state_with(&desk.pool, &env);
    assert!(state.settings.identity.is_some());
    assert!(state.settings.identity.as_ref().unwrap().api_key.is_none());

    let notice = state
        .settings
        .identity_api_key_notice()
        .expect("missing IDENTITY_API_KEY must produce a notice");
    assert!(
        notice.contains("IDENTITY_API_KEY"),
        "the notice must name IDENTITY_API_KEY: {notice}"
    );

    // Verify when key is present, no notice is produced
    let env_with_key = identity_env(&identity.url());
    let state_with_key = state_with(&desk.pool, &env_with_key);
    assert_eq!(
        state_with_key.settings.identity_api_key_notice(),
        None,
        "when IDENTITY_API_KEY is present, no notice is emitted"
    );

    let app = app(state).await;

    let _user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Wichai",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);

    let reply = app.send(login_req("wichai")).await;
    assert_eq!(reply.status, 200);
    assert!(!reply.session().is_empty());

    desk.drop_schema().await;
}
