//! BT-5 follow-up (#4746): when `bind_on_sign_in` fails during `POST /api/auth/otp/verify`, the
//! handler ends the newly created Identity session before answering `500 serverError` and sets no
//! session cookie.

#[macro_use]
mod auth_http_common;
mod common;
mod identity_signin_common;

use auth_http_common::Desk;
use chrono::Utc;
use identity_signin_common::*;
use serde_json::json;
use xc_customer::application::consent_service::{ConsentService, Registration};
use xc_customer::domain::consent::Purpose;

const PHONE: &str = "+66812345678";
const CHALLENGE: &str = "c0ffee00-0000-4000-8000-0000000000b5";
const TABLET: &str = "7ab1e700-0000-4000-8000-000000000001";
const TABLET_TOKEN: &str = "xcbt_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const TABLET_COOKIE: &str = "__Host-xc_tablet";

#[actix_web::test]
async fn bind_failure_on_otp_verify_ends_the_new_session_and_returns_500() {
    common::install_test_pii_keys();
    let desk = Desk::start().await;
    let identity = FakeIdentity::start();
    let app = app(state_with(&desk.pool, &identity_env(&identity.url()))).await;
    let org = organization(&desk.pool).await;
    ConsentService::pg(desk.pool.clone())
        .publish(&org, None, Purpose::Terms, "th", "ข้อความยินยอม", Utc::now())
        .await
        .expect("publish TERMS");

    let user = bound_user(&desk.pool, SUB_A, "ACTIVE", "Nok", &[("CUSTOMER", None)]).await;
    ConsentService::pg(desk.pool.clone())
        .register(
            &org,
            Registration {
                user_id: user,
                display_name: "Nok".to_owned(),
                lang: "th".to_owned(),
                marketing: vec![],
                evidence_ref: "test".to_owned(),
            },
            Utc::now(),
        )
        .await
        .expect("CI-6 register");

    sqlx::query(
        "INSERT INTO branch_tablets (id, branch_id, name, created_at, created_by, token_hash, paired_at) \
         VALUES ($1::uuid, 'MAIN', 'front tablet', now(), 'admin', $2, now())",
    )
    .bind(TABLET)
    .bind(sha256_hex(TABLET_TOKEN))
    .execute(&desk.pool)
    .await
    .expect("insert paired tablet");

    // Force `bind_on_sign_in` to error when binding a customer session to the tablet.
    sqlx::query(
        "ALTER TABLE branch_tablets ADD CONSTRAINT fail_bind CHECK (customer_session_hash IS NULL)",
    )
    .execute(&desk.pool)
    .await
    .expect("add failing constraint");

    let mut login = login_body(VALID_A, REFRESH_1);
    let o = login.as_object_mut().unwrap();
    o.insert("phoneVerified".to_owned(), json!(true));
    o.insert("phone".to_owned(), json!(PHONE));
    identity.answer(
        "/auth/otp/verify",
        200,
        &json!({ "verified": true, "login": login, "registered": false }).to_string(),
    );
    identity.answer("/auth/logout", 200, "{}");

    let mut req = Req::post(
        "/api/auth/otp/verify",
        json!({
            "challengeId": CHALLENGE,
            "code": "123456",
            "register": { "displayName": "Somchai J.", "lang": "th", "marketing": [] }
        }),
    );
    req.cookie = Some(format!("{TABLET_COOKIE}={TABLET_TOKEN}"));
    let reply = app.send(req).await;

    assert_eq!(reply.status, 500, "{}", reply.text);
    assert_eq!(reply.refusal().0, "serverError");
    assert!(
        !reply.cookies.contains_key(SESSION_COOKIE),
        "no session cookie set on 500: {:?}",
        reply.cookies
    );
    assert_eq!(
        all_sessions(&desk.pool).await,
        0,
        "new session row was ended and deleted"
    );
    let logouts = identity.received("/auth/logout");
    assert_eq!(logouts.len(), 1, "Identity received logout");
    assert_eq!(logouts[0].json()["refresh_token"], json!(REFRESH_1));

    // Without a tablet cookie, sign-in remains unchanged and authenticates at `/api/auth/session`.
    let plain = app
        .send(Req::post(
            "/api/auth/otp/verify",
            json!({
                "challengeId": CHALLENGE,
                "code": "123456",
                "register": { "displayName": "Somchai J.", "lang": "th", "marketing": [] }
            }),
        ))
        .await;
    assert_eq!(plain.status, 200, "{}", plain.text);
    let session_check = app
        .send(Req::post("/api/auth/session", json!({})).with_session(&plain.session()))
        .await;
    assert_eq!(session_check.status, 200, "{}", session_check.text);
    assert!(session_check.json()["user"].is_object());

    desk.drop_schema().await;
}
