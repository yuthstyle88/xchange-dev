//! OR-3 (#550 design `docs/design/blueprint/org-roles-branches.md` §5, §10 "OR-3" up-front tests): sign-in
//! through Identity — the BFF routes under `/api/auth/*`, the one session cookie, `user_sessions`, and the session
//! reaching the old route code (a shift opened by a user signed in the new way).
//!
//! Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Amended by #1157 (claude/08f7f46f, the owner's order "ไม่มีระบบเชิญสำหรับพนักงาน"): the code-claim cases
//! and the claim route's 503 row are removed with the route, as the ticket lists; the not-bound sentence no longer
//! points to a one-time code. Every other case is unchanged.
//!
//! Identity is faked over real HTTP (`identity_signin_common::FakeIdentity`); nothing here reaches a real
//! Identity. The token checks run through I-2's verifier (#649), fed by the fake's JWKS.
//!
//! `TEST_DATABASE_URL` must name a database whose name ends in `_test`; each test gets its own schema.

mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use identity_signin_common::*;
use serde_json::json;
use sqlx::PgPool;
use xchange::config::Settings;

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const NOT_BOUND: &str = "บัญชีนี้ยังไม่มีผู้ใช้ใน XCHANGE ให้เจ้าของร้านสร้างบัญชีให้";
const DISABLED: &str = "บัญชีถูกระงับ";

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

async fn refused_without_session(
    pool: &PgPool,
    reply: &Reply,
    status: u16,
    code: &str,
    message: &str,
) {
    assert_eq!(reply.status, status, "{}", reply.text);
    assert_eq!(
        reply.refusal(),
        (code.to_owned(), message.to_owned()),
        "{}",
        reply.text
    );
    assert!(
        reply.sets_no_session(),
        "no session cookie: {:?}",
        reply.cookies
    );
    assert_eq!(all_sessions(pool).await, 0, "no user_sessions row");
}

// ── signing in ────────────────────────────────────────────────────────────

/// §5.1: Identity's token verifies, its `sub` names an ACTIVE user → one `user_sessions` row, the one
/// cookie, the two old cookies cleared; Identity got the client credentials, the API key and the browser's
/// address; the browser got none of the secrets.
#[actix_web::test]
async fn a_signin_through_identity_sets_the_one_session_cookie_and_clears_the_old_two() {
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

    let reply = app.send(login("+66812345678")).await;

    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.cache_control.as_deref(), Some("no-store"));
    let body = reply.json();
    assert_eq!(body["user"]["id"], json!(user));
    assert_eq!(body["user"]["name"], json!("Nok"));
    assert_eq!(
        body["user"]["organizationId"],
        json!(organization(&desk.pool).await)
    );
    assert_eq!(
        body["user"]["roles"],
        json!([{ "role": "TELLER", "branchId": "MAIN" }])
    );

    let token = reply.session();
    assert!(
        token.len() == 64
            && token
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "the session cookie is 64 lower-case hex characters: {token:?}"
    );
    let cookie = &reply.cookies[SESSION_COOKIE];
    assert_eq!(
        cookie.attributes,
        "Path=/; HttpOnly; Secure; SameSite=Strict"
    );
    assert_eq!(cookie.max_age, SESSION_SECONDS);
    for old in [OWNER_COOKIE, STAFF_COOKIE] {
        let cleared = reply
            .cookies
            .get(old)
            .unwrap_or_else(|| panic!("{old} is cleared: {:?}", reply.cookies));
        assert_eq!((cleared.value.as_str(), cleared.max_age), ("", 0), "{old}");
    }

    for secret in [
        CLIENT_SECRET,
        API_KEY,
        VALID_A,
        REFRESH_1,
        "refresh-secret-one",
    ] {
        assert!(
            !reply.text.contains(secret),
            "the browser never sees {secret:?}"
        );
    }

    let sent = identity.received("/auth/login");
    assert_eq!(sent.len(), 1, "one login at Identity");
    let sent = &sent[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(
        sent.json(),
        json!({ "identifier": "+66812345678", "password": "pw-from-the-browser-01" })
    );
    assert_eq!(sent.header("x-client-id"), Some(CLIENT_ID));
    assert_eq!(sent.header("x-client-secret"), Some(CLIENT_SECRET));
    assert_eq!(sent.header("x-api-key"), Some(API_KEY));
    assert_eq!(sent.header("x-real-ip"), Some(BROWSER_ADDRESS));
    assert_eq!(
        sent.header("x-identity-token-transport"),
        None,
        "v3 takes the refresh token in the body (Identity's default transport)"
    );

    let (row_user, row_version, user_version, seconds_left, sealed): (
        String,
        i32,
        i32,
        f64,
        Option<Vec<u8>>,
    ) = sqlx::query_as(
        "SELECT s.user_id::text, s.version, u.version, \
                    EXTRACT(EPOCH FROM s.expires - now())::float8, s.refresh_token_enc \
             FROM user_sessions s JOIN users u ON u.id = s.user_id WHERE s.hash = $1",
    )
    .bind(sha256_hex(&token))
    .fetch_one(&desk.pool)
    .await
    .expect("the session row is keyed by the SHA-256 of the cookie, never the cookie itself");
    assert_eq!(row_user, user);
    assert_eq!(row_version, user_version, "the row carries users.version");
    assert!(
        (SESSION_SECONDS as f64 - 120.0..=SESSION_SECONDS as f64 + 5.0).contains(&seconds_left),
        "the row expires with the cookie: {seconds_left}"
    );
    let sealed = sealed.expect("the refresh token is kept, encrypted");
    assert!(!sealed.is_empty());
    for plain in [
        REFRESH_1.as_bytes(),
        b"refresh-secret-one-0123456789".as_slice(),
    ] {
        assert!(
            !sealed.windows(plain.len()).any(|w| w == plain),
            "refresh_token_enc does not hold the refresh token in the clear"
        );
    }
    assert_eq!(all_sessions(&desk.pool).await, 1);
    desk.drop_schema().await;
}

/// §10 OR-3: a JWT whose `aud` is not `xchange` is refused (I-2's verifier); nothing is signed in.
#[actix_web::test]
async fn a_token_with_the_wrong_audience_is_refused() {
    let (desk, identity, app) = setup().await;
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", WRONG_AUD_A, REFRESH_1);
    let reply = app.send(login("nok")).await;
    refused_without_session(&desk.pool, &reply, 503, "identityUnavailable", UNAVAILABLE).await;
    desk.drop_schema().await;
}

/// §10 OR-3: a JWT signed by another key under the published kid is refused.
#[actix_web::test]
async fn a_token_with_a_wrong_signature_is_refused() {
    let (desk, identity, app) = setup().await;
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", BAD_SIG_A, REFRESH_1);
    let reply = app.send(login("nok")).await;
    refused_without_session(&desk.pool, &reply, 503, "identityUnavailable", UNAVAILABLE).await;
    desk.drop_schema().await;
}

/// §10 OR-3: an expired JWT is refused.
#[actix_web::test]
async fn an_expired_token_is_refused() {
    let (desk, identity, app) = setup().await;
    bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", EXPIRED_A, REFRESH_1);
    let reply = app.send(login("nok")).await;
    refused_without_session(&desk.pool, &reply, 503, "identityUnavailable", UNAVAILABLE).await;
    desk.drop_schema().await;
}

/// §5.1: errors map by Identity's error **code**, never by its HTTP status. Each user-side code → its v3
/// status, code and sentence. (Identity's codes: `108-platform-services` `crates/auth/src/api/error.rs`.)
#[actix_web::test]
async fn each_identity_error_code_maps_to_its_sentence() {
    let (desk, identity, app) = setup().await;
    let rows: &[(u16, &str, u16, &str, &str)] = &[
        (
            401,
            "invalid_credentials",
            401,
            "invalidCredentials",
            "ชื่อผู้ใช้หรือรหัสผ่านไม่ถูกต้อง",
        ),
        (
            423,
            "account_locked",
            423,
            "accountLocked",
            "บัญชีถูกล็อกชั่วคราว กรุณาลองใหม่ภายหลัง",
        ),
        (
            429,
            "rate_limited",
            429,
            "tooManyAttempts",
            "ลองหลายครั้งเกินไป กรุณารอสักครู่แล้วลองใหม่",
        ),
        (
            429,
            "too_many_attempts",
            429,
            "tooManyAttempts",
            "ลองหลายครั้งเกินไป กรุณารอสักครู่แล้วลองใหม่",
        ),
        (
            403,
            "identity_disabled",
            403,
            "identityDisabled",
            "บัญชีนี้ถูกระงับที่ระบบยืนยันตัวตน",
        ),
        (401, "invalid_code", 401, "invalidCode", "รหัสยืนยันไม่ถูกต้อง"),
        (
            410,
            "challenge_expired",
            410,
            "codeExpired",
            "รหัสยืนยันหมดอายุ กรุณาขอรหัสใหม่",
        ),
        (
            404,
            "challenge_not_found",
            410,
            "codeExpired",
            "รหัสยืนยันหมดอายุ กรุณาขอรหัสใหม่",
        ),
        (
            400,
            "invalid_challenge_state",
            410,
            "codeExpired",
            "รหัสยืนยันหมดอายุ กรุณาขอรหัสใหม่",
        ),
        (401, "invalid_refresh_token", 401, "signIn", SIGN_IN),
        (401, "refresh_token_expired", 401, "signIn", SIGN_IN),
        (401, "refresh_token_reuse", 401, "signIn", SIGN_IN),
        (401, "session_inactive", 401, "signIn", SIGN_IN),
        (404, "session_not_found", 401, "signIn", SIGN_IN),
        (
            409,
            "username_taken",
            409,
            "alreadyRegistered",
            "มีบัญชีนี้อยู่แล้ว กรุณาเข้าสู่ระบบ",
        ),
        (
            409,
            "email_taken",
            409,
            "alreadyRegistered",
            "มีบัญชีนี้อยู่แล้ว กรุณาเข้าสู่ระบบ",
        ),
        (
            409,
            "phone_taken",
            409,
            "alreadyRegistered",
            "มีบัญชีนี้อยู่แล้ว กรุณาเข้าสู่ระบบ",
        ),
        (
            422,
            "phone_required",
            422,
            "phoneRequired",
            "กรุณากรอกเบอร์โทรศัพท์",
        ),
        (
            422,
            "invalid_phone",
            422,
            "invalidPhone",
            "เบอร์โทรศัพท์ไม่ถูกต้อง",
        ),
        (
            422,
            "unsupported_country_code",
            422,
            "invalidPhone",
            "เบอร์โทรศัพท์ไม่ถูกต้อง",
        ),
        (
            422,
            "password_policy_violation",
            422,
            "weakPassword",
            "รหัสผ่านไม่ผ่านเงื่อนไขความปลอดภัย",
        ),
        (
            400,
            "missing_identifier",
            400,
            "invalidRequest",
            "คำขอไม่ถูกต้อง",
        ),
    ];
    for (identity_status, identity_code, status, code, message) in rows {
        identity.answer(
            "/auth/login",
            *identity_status,
            &json!({ "error": identity_code }).to_string(),
        );
        let reply = app.send(login("nok")).await;
        assert_eq!(reply.status, *status, "{identity_code}: {}", reply.text);
        assert_eq!(
            reply.refusal(),
            ((*code).to_owned(), (*message).to_owned()),
            "{identity_code}"
        );
        assert!(reply.sets_no_session(), "{identity_code}");
    }
    assert_eq!(all_sessions(&desk.pool).await, 0);
    desk.drop_schema().await;
}

/// §5.1: an unknown code, a caller-side code (v3's own fault: client credentials, API key), an unreadable
/// body and any 5xx all become 503 — even a 5xx that carries a user-side code.
#[actix_web::test]
async fn an_unknown_or_caller_side_identity_answer_is_503() {
    let (desk, identity, app) = setup().await;
    let rows: &[(u16, &str)] = &[
        (401, r#"{"error":"some_code_v3_has_never_seen"}"#),
        (401, r#"{"error":"invalid_client_credentials"}"#),
        (400, r#"{"error":"malformed_client_credentials"}"#),
        (404, r#"{"error":"client_application_not_found"}"#),
        (403, r#"{"error":"client_disabled"}"#),
        (404, r#"{"error":"realm_not_found"}"#),
        (401, ""),
        (500, r#"{"error":"internal_error"}"#),
        (502, "<html>bad gateway</html>"),
        (503, r#"{"error":"invalid_credentials"}"#),
        (200, "this is not json"),
        (200, r#"{"identityId":"no-token-here"}"#),
    ];
    for (status, body) in rows {
        identity.answer("/auth/login", *status, body);
        let reply = app.send(login("nok")).await;
        assert_eq!(reply.status, 503, "{status} {body}: {}", reply.text);
        assert_eq!(
            reply.refusal(),
            ("identityUnavailable".to_owned(), UNAVAILABLE.to_owned()),
            "{status} {body}"
        );
        assert!(reply.sets_no_session(), "{status} {body}");
    }
    assert_eq!(all_sessions(&desk.pool).await, 0);
    desk.drop_schema().await;
}

/// §5.1: Identity unreachable → 503.
#[actix_web::test]
async fn an_unreachable_identity_is_503() {
    let desk = Desk::start().await;
    let url = format!("http://127.0.0.1:{}", closed_port());
    let app = app(state_with(&desk.pool, &identity_env(&url))).await;
    let reply = app.send(login("nok")).await;
    refused_without_session(&desk.pool, &reply, 503, "identityUnavailable", UNAVAILABLE).await;
    desk.drop_schema().await;
}

/// An install with no Identity settings answers 503 on every route that needs Identity, and starts.
#[actix_web::test]
async fn without_identity_settings_the_routes_that_need_identity_are_503() {
    let desk = Desk::start().await;
    let app = app(state_with(&desk.pool, &[])).await;
    for path in [
        "/api/auth/login",
        "/api/auth/otp/request",
        "/api/auth/otp/verify",
    ] {
        let reply = app.send(Req::post(path, json!({}))).await;
        assert_eq!(reply.status, 503, "{path}: {}", reply.text);
        assert_eq!(reply.refusal().0, "identityUnavailable", "{path}");
    }
    desk.drop_schema().await;
}

/// §10 OR-3: a valid `sub` that no v3 user carries → `signIn`; nothing is signed in.
#[actix_web::test]
async fn a_valid_subject_with_no_user_is_sign_in() {
    let (desk, identity, app) = setup().await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(login("stranger")).await;
    refused_without_session(&desk.pool, &reply, 401, "signIn", NOT_BOUND).await;
    desk.drop_schema().await;
}

/// §10 OR-3, §5.3: a DISABLED user is refused even though Identity signed them in.
#[actix_web::test]
async fn a_disabled_user_is_refused() {
    let (desk, identity, app) = setup().await;
    bound_user(
        &desk.pool,
        SUB_A,
        "DISABLED",
        "Gone",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(login("gone")).await;
    refused_without_session(&desk.pool, &reply, 403, "userDisabled", DISABLED).await;
    desk.drop_schema().await;
}

/// The same-origin rule of every v3 write (`api/src/presentation/extractors/origin.rs`) holds on the BFF:
/// a cross-site sign-in never reaches Identity.
#[actix_web::test]
async fn a_cross_site_signin_is_refused_before_identity() {
    let (desk, identity, app) = setup().await;
    identity.login_answers("/auth/login", VALID_A, REFRESH_1);
    let reply = app.send(login("nok").cross_site()).await;
    assert_eq!(reply.status, 400, "{}", reply.text);
    assert_eq!(reply.refusal().0, "invalidRequest");
    assert!(identity.calls().is_empty(), "Identity was not called");
    desk.drop_schema().await;
}

// ── the session ───────────────────────────────────────────────────────────

fn session_of(token: &str) -> Req {
    Req::post("/api/auth/session", json!({})).with_session(token)
}

/// §5.1, §10 OR-3: `users.version` moving (any role or status change) ends that user's sessions.
#[actix_web::test]
async fn a_users_version_change_ends_the_session() {
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

    let before = app.send(session_of(&token)).await;
    assert_eq!(before.status, 200, "{}", before.text);
    assert_eq!(before.json()["user"]["id"], json!(user));

    sqlx::query("UPDATE users SET version = version + 1 WHERE id = $1::uuid")
        .bind(&user)
        .execute(&desk.pool)
        .await
        .expect("bump users.version");

    let after = app.send(session_of(&token)).await;
    assert_eq!(after.status, 200, "{}", after.text);
    assert_eq!(after.json(), json!({ "user": null }), "the session ended");
    desk.drop_schema().await;
}

/// A session reads nobody once the user is DISABLED, even without a version change.
#[actix_web::test]
async fn a_session_of_a_user_disabled_since_reads_nobody() {
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
    sqlx::query("UPDATE users SET status = 'DISABLED' WHERE id = $1::uuid")
        .bind(&user)
        .execute(&desk.pool)
        .await
        .expect("disable");
    assert_eq!(
        app.send(session_of(&token)).await.json(),
        json!({ "user": null })
    );
    desk.drop_schema().await;
}

/// No cookie, an unknown cookie → `{"user": null}`.
#[actix_web::test]
async fn no_session_reads_nobody() {
    let (desk, _identity, app) = setup().await;
    let none = app.send(Req::post("/api/auth/session", json!({}))).await;
    assert_eq!((none.status, none.json()), (200, json!({ "user": null })));
    let unknown = app.send(session_of(&"ab".repeat(32))).await;
    assert_eq!(
        (unknown.status, unknown.json()),
        (200, json!({ "user": null }))
    );
    desk.drop_schema().await;
}

/// Logout ends the session here, clears the cookies, and revokes the refresh token at Identity.
#[actix_web::test]
async fn logout_ends_the_session_and_revokes_the_refresh_token_at_identity() {
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
    let sent = identity.received("/auth/logout");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].json(), json!({ "refresh_token": REFRESH_1 }));
    assert_eq!(sent[0].header("x-client-id"), Some(CLIENT_ID));
    assert_eq!(sent[0].header("x-api-key"), Some(API_KEY));
    assert_eq!(
        app.send(session_of(&token)).await.json(),
        json!({ "user": null })
    );
    desk.drop_schema().await;
}

/// Refresh: Identity takes the stored refresh token; when Identity refuses it, the v3 session ends.
#[actix_web::test]
async fn a_refresh_identity_refuses_ends_the_session() {
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
    assert_eq!(
        identity.received("/auth/refresh")[0].json(),
        json!({ "refresh_token": REFRESH_1 })
    );
    desk.drop_schema().await;
}

/// Refresh accepted: the new refresh token replaces the stored one and the session lives on.
#[actix_web::test]
async fn a_refresh_identity_accepts_keeps_the_session_with_the_new_refresh_token() {
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
    identity.answer("/auth/logout", 204, "");
    let token = app.send(login("nok")).await.session();

    let reply = app
        .send(Req::post("/api/auth/refresh", json!({})).with_session(&token))
        .await;
    assert_eq!(reply.status, 200, "{}", reply.text);
    assert_eq!(reply.json()["user"]["id"], json!(user));
    assert_eq!(
        app.send(session_of(&token)).await.json()["user"]["id"],
        json!(user)
    );

    app.send(Req::post("/api/auth/logout", json!({})).with_session(&token))
        .await;
    assert_eq!(
        identity.received("/auth/logout")[0].json(),
        json!({ "refresh_token": REFRESH_2 }),
        "the stored refresh token is the newest one"
    );
    desk.drop_schema().await;
}

// ── OTP and registration (§5.1 forwarded routes) ──────────────────────────

/// OTP request is forwarded; OTP verify that returns a login signs in like a password login, and v3 never
/// asks Identity to register on verify.
#[actix_web::test]
async fn otp_request_is_forwarded_and_a_verified_otp_signs_in() {
    let (desk, identity, app) = setup().await;
    let user = bound_user(
        &desk.pool,
        SUB_A,
        "ACTIVE",
        "Nok",
        &[("TELLER", Some("MAIN"))],
    )
    .await;
    identity.answer(
        "/auth/otp/request",
        200,
        r#"{"challengeId":"c0ffee00-0000-4000-8000-000000000001","expiresAt":4102444800}"#,
    );
    identity.answer(
        "/auth/otp/verify",
        200,
        &json!({ "verified": true, "login": login_body(VALID_A, REFRESH_1) }).to_string(),
    );

    let asked = app
        .send(Req::post(
            "/api/auth/otp/request",
            json!({ "destination": "+66812345678", "channel": "sms" }),
        ))
        .await;
    assert_eq!(asked.status, 200, "{}", asked.text);
    assert_eq!(
        asked.json(),
        json!({ "challengeId": "c0ffee00-0000-4000-8000-000000000001", "expiresAt": 4102444800u64 })
    );
    assert_eq!(
        identity.received("/auth/otp/request")[0].json(),
        json!({ "destination": "+66812345678", "channel": "sms" })
    );

    let verified = app
        .send(Req::post(
            "/api/auth/otp/verify",
            json!({ "challengeId": "c0ffee00-0000-4000-8000-000000000001", "code": "123456" }),
        ))
        .await;
    assert_eq!(verified.status, 200, "{}", verified.text);
    assert_eq!(verified.json()["user"]["id"], json!(user));
    verified.session();
    assert_eq!(
        identity.received("/auth/otp/verify")[0].json(),
        json!({ "challengeId": "c0ffee00-0000-4000-8000-000000000001", "code": "123456",
                "registerIfAbsent": false })
    );
    desk.drop_schema().await;
}

// ── the session reaches the old route code: retired with CA-10 ────────────
// CA-10 #626 retires the direct `adminOpenShift` (cash-shift-transfer.md §6), so the three tests that opened a
// shift through it as an Identity-signed-in owner, branch manager and marketing user are removed by CA-10's
// ticket author (claude/c4d72c). On the count path that replaces it, the session's `users.id` is what a count
// records as its `counted_by` (`v3_cash_commands_test.rs`), and a shift opened by a verified count names it
// as `teller_id` (CA-3).

// ── settings ──────────────────────────────────────────────────────────────

fn owned(pairs: &[(&str, String)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// The five `IDENTITY_*` variables are read (the setting is visible) and the secrets never print.
#[test]
fn identity_settings_are_read_and_their_secrets_never_print() {
    let settings = Settings::load_from(
        &common::config_dir(),
        owned(&identity_env("http://127.0.0.1:8095")),
    )
    .expect("the settings load");
    let printed = format!("{settings:?}");
    assert!(
        printed.contains("http://127.0.0.1:8095"),
        "the base URL is read: {printed}"
    );
    assert!(printed.contains(CLIENT_ID), "the client id is read");
    assert!(
        !printed.contains(CLIENT_SECRET),
        "the client secret never prints"
    );
    assert!(!printed.contains(API_KEY), "the API key never prints");
}

/// Some `IDENTITY_*` set and others not is a broken install: the server refuses to start.
#[test]
fn identity_settings_set_in_part_refuse_to_load() {
    for missing in [
        "IDENTITY_BASE_URL",
        "IDENTITY_CLIENT_ID",
        "IDENTITY_CLIENT_SECRET",
        "IDENTITY_AUDIENCE",
        "IDENTITY_ISSUER",
    ] {
        let env: Vec<(String, String)> = owned(&identity_env("http://127.0.0.1:8095"))
            .into_iter()
            .filter(|(k, _)| k != missing)
            .collect();
        assert!(
            Settings::load_from(&common::config_dir(), env).is_err(),
            "{missing} missing must not load"
        );
    }
    let without_key: Vec<(String, String)> = owned(&identity_env("http://127.0.0.1:8095"))
        .into_iter()
        .filter(|(k, _)| k != "IDENTITY_API_KEY")
        .collect();
    assert!(
        Settings::load_from(&common::config_dir(), without_key).is_ok(),
        "IDENTITY_API_KEY is optional"
    );
}
