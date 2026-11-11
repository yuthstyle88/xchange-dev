//! #6268 — Customer chooses OTP channel (email or SMS) on the sign-in screen.
//! Pins O1 (default `["email", "sms"]`), O4 (owner allows only email -> SMS refused with 400
//! `invalidRequest`), and O5 (invalid channel -> 400 `invalidRequest`).
//! #6286 (decision A): O1 and O4 run on a migrated schema and set `customer_app_settings.otp_channels`;
//! the constant and the injected seam are gone.

mod auth_http_common;
mod common;
mod identity_signin_common;

use actix_web::{App as ActixApp, http::header, test};
use auth_http_common::Desk;
use identity_signin_common::{
    BROWSER_ADDRESS, FakeIdentity, HOST, ORIGIN, identity_env, state_with,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_auth::domain::identity::identity_client::IdentityLoginResponse;
use xchange::application::customer_otp_sign_in::proven_contacts;
use xchange::application::otp_challenge_seal::{OTP_COOKIE, OtpChallengeSeal, OtpChannel};

const CHALLENGE: &str = "c0ffee00-0000-4000-8000-000000006268";

fn lazy_pool() -> PgPool {
    PgPool::connect_lazy("postgres://localhost/xchange_test").expect("lazy pool")
}

/// Sets the seeded organisation's `otp_channels` (`None` = not set).
async fn store_channels(pool: &PgPool, channels: Option<&[&str]>) {
    let channels = channels.map(|c| c.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    sqlx::query(
        "UPDATE customer_app_settings SET otp_channels = $1 \
         WHERE organization_id = (SELECT id FROM organizations WHERE code = 'XC')",
    )
    .bind(channels)
    .execute(pool)
    .await
    .expect("store otp_channels");
}

#[actix_web::test]
async fn o1_default_offers_email_and_sms() {
    common::install_test_pii_keys();
    let desk = Desk::start().await;
    store_channels(&desk.pool, None).await;
    let pool = desk.pool.clone();
    let identity = FakeIdentity::start();
    identity.answer(
        "/auth/otp/request",
        200,
        &json!({ "challengeId": CHALLENGE, "expiresAt": 4102444800u64 }).to_string(),
    );
    let state = state_with(&pool, &identity_env(&identity.url()));
    let app = test::init_service(
        ActixApp::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/auth/otp/channels")
        .insert_header((header::HOST, HOST))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(
        body,
        json!({
            "channels": ["email", "sms"],
            "source": "default",
        })
    );

    for (channel, destination) in [("email", "customer@example.com"), ("sms", "+66812345678")] {
        let req = test::TestRequest::post()
            .uri("/api/auth/otp/request")
            .insert_header((header::HOST, HOST))
            .insert_header(("x-forwarded-proto", "https"))
            .insert_header(("x-real-ip", BROWSER_ADDRESS))
            .insert_header((header::ORIGIN, ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .set_payload(json!({ "destination": destination, "channel": channel }).to_string())
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status().as_u16(),
            200,
            "channel {channel} allowed by default"
        );
    }
    assert_eq!(identity.received("/auth/otp/request").len(), 2);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn o4_owner_allows_only_email_refuses_sms_with_400_invalid_request() {
    common::install_test_pii_keys();
    let desk = Desk::start().await;
    store_channels(&desk.pool, Some(&["email"])).await;
    let pool = desk.pool.clone();
    let identity = FakeIdentity::start();
    identity.answer(
        "/auth/otp/request",
        200,
        &json!({ "challengeId": CHALLENGE, "expiresAt": 4102444800u64 }).to_string(),
    );
    let state = state_with(&pool, &identity_env(&identity.url()));
    let app = test::init_service(
        ActixApp::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
    )
    .await;

    let req = test::TestRequest::get()
        .uri("/api/auth/otp/channels")
        .insert_header((header::HOST, HOST))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(
        body,
        json!({
            "channels": ["email"],
            "source": "set",
        })
    );

    // SMS is disallowed -> 400 invalidRequest, Identity is not called.
    let sms_req = test::TestRequest::post()
        .uri("/api/auth/otp/request")
        .insert_header((header::HOST, HOST))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-real-ip", BROWSER_ADDRESS))
        .insert_header((header::ORIGIN, ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .set_payload(json!({ "destination": "+66812345678", "channel": "sms" }).to_string())
        .to_request();
    let sms_resp = test::call_service(&app, sms_req).await;
    assert_eq!(sms_resp.status().as_u16(), 400);
    let sms_body: Value = serde_json::from_slice(&test::read_body(sms_resp).await).unwrap();
    assert_eq!(sms_body["error"]["code"], "invalidRequest");
    assert_eq!(sms_body["error"]["message"], "คำขอไม่ถูกต้อง");
    assert!(identity.received("/auth/otp/request").is_empty());

    // Email is allowed -> 200.
    let email_req = test::TestRequest::post()
        .uri("/api/auth/otp/request")
        .insert_header((header::HOST, HOST))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-real-ip", BROWSER_ADDRESS))
        .insert_header((header::ORIGIN, ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .set_payload(
            json!({ "destination": "customer@example.com", "channel": "email" }).to_string(),
        )
        .to_request();
    let email_resp = test::call_service(&app, email_req).await;
    assert_eq!(email_resp.status().as_u16(), 200);
    assert_eq!(identity.received("/auth/otp/request").len(), 1);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn o5_invalid_channel_refused_with_400_invalid_request() {
    common::install_test_pii_keys();
    let pool = lazy_pool();
    let identity = FakeIdentity::start();
    let state = state_with(&pool, &identity_env(&identity.url()));
    let app = test::init_service(
        ActixApp::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
    )
    .await;

    for bad_channel in ["fax", "whatsapp", "voice", ""] {
        let req = test::TestRequest::post()
            .uri("/api/auth/otp/request")
            .insert_header((header::HOST, HOST))
            .insert_header(("x-forwarded-proto", "https"))
            .insert_header(("x-real-ip", BROWSER_ADDRESS))
            .insert_header((header::ORIGIN, ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .set_payload(
                json!({ "destination": "customer@example.com", "channel": bad_channel })
                    .to_string(),
            )
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status().as_u16(),
            400,
            "channel {bad_channel:?} must be refused with 400"
        );
        let body: Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
        assert_eq!(body["error"]["code"], "invalidRequest");
        assert_eq!(body["error"]["message"], "คำขอไม่ถูกต้อง");
    }
    assert!(identity.received("/auth/otp/request").is_empty());
}

#[actix_web::test]
async fn sms_issued_challenge_cannot_verify_email_destination() {
    common::install_test_pii_keys();
    let keys = xc_customer::get_pii_keys().expect("test pii keys");
    let desk = Desk::start().await;
    let pool = desk.pool.clone();
    let identity = FakeIdentity::start();
    identity.answer(
        "/auth/otp/request",
        200,
        &json!({ "challengeId": CHALLENGE, "expiresAt": 4102444800u64 }).to_string(),
    );
    let state = state_with(&pool, &identity_env(&identity.url()));
    let app = test::init_service(
        ActixApp::new().configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
    )
    .await;

    let sms_req = test::TestRequest::post()
        .uri("/api/auth/otp/request")
        .insert_header((header::HOST, HOST))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-real-ip", BROWSER_ADDRESS))
        .insert_header((header::ORIGIN, ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .set_payload(json!({ "destination": "+66812345678", "channel": "sms" }).to_string())
        .to_request();
    let sms_resp = test::call_service(&app, sms_req).await;
    assert_eq!(sms_resp.status().as_u16(), 200);

    let set_cookie = sms_resp
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .expect("SMS request sets OTP seal cookie");
    let raw_seal = set_cookie
        .split(';')
        .next()
        .and_then(|kv| kv.strip_prefix(&format!("{OTP_COOKIE}=")))
        .expect("OTP_COOKIE value");

    let opened = OtpChallengeSeal::open(raw_seal, &keys).expect("valid SMS seal");
    assert_eq!(opened.channel, OtpChannel::Sms);
    assert_eq!(opened.destination, "+66812345678");

    // 1. Forging the cookie to claim an email destination with the SMS challenge's MAC fails to open.
    let parts: Vec<&str> = raw_seal.split('.').collect();
    let forged_email_seal = format!(
        "v1.email.{}.{}.{}",
        base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            b"customer@example.com"
        ),
        parts[3],
        parts[4],
    );
    assert_eq!(OtpChallengeSeal::open(&forged_email_seal, &keys), None);

    // 2. Keeping the valid SMS seal does not prove any email contact when verifying that challenge.
    let login: IdentityLoginResponse = serde_json::from_value(json!({
        "identityId": "sub-1",
        "access_token": "sess-1",
        "phone": "+66812345678",
        "emailVerified": true,
        "phoneVerified": true,
    }))
    .unwrap();
    let contacts = proven_contacts(&login, Some(&opened), CHALLENGE);
    assert!(
        contacts.iter().all(|c| !matches!(
            c,
            xchange::application::customer_binder::ProvenContact::Email(_)
        )),
        "SMS seal must never prove an email destination: {contacts:?}"
    );
    desk.drop_schema().await;
}
