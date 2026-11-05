//! #6268 — Customer chooses OTP channel (email or SMS) on the sign-in screen.
//! Pins O1 (default `["email", "sms"]`), O4 (owner allows only email -> SMS refused with 400
//! `invalidRequest`), and O5 (invalid channel -> 400 `invalidRequest`).

mod common;
mod identity_signin_common;

use actix_web::{App as ActixApp, http::header, test, web};
use identity_signin_common::{BROWSER_ADDRESS, FakeIdentity, HOST, ORIGIN, identity_env, state_with};
use serde_json::{Value, json};
use sqlx::PgPool;
use xchange::presentation::http::auth::{AllowedOtpChannels, DEFAULT_OTP_CHANNELS};

const CHALLENGE: &str = "c0ffee00-0000-4000-8000-000000006268";

fn lazy_pool() -> PgPool {
    PgPool::connect_lazy("postgres://localhost/xchange_test").expect("lazy pool")
}

#[actix_web::test]
async fn o1_default_offers_email_and_sms() {
    common::install_test_pii_keys();
    assert_eq!(DEFAULT_OTP_CHANNELS, &["email", "sms"]);

    let pool = lazy_pool();
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
        assert_eq!(resp.status().as_u16(), 200, "channel {channel} allowed by default");
    }
    assert_eq!(identity.received("/auth/otp/request").len(), 2);
}

#[actix_web::test]
async fn o4_owner_allows_only_email_refuses_sms_with_400_invalid_request() {
    common::install_test_pii_keys();
    let pool = lazy_pool();
    let identity = FakeIdentity::start();
    identity.answer(
        "/auth/otp/request",
        200,
        &json!({ "challengeId": CHALLENGE, "expiresAt": 4102444800u64 }).to_string(),
    );
    let state = state_with(&pool, &identity_env(&identity.url()));
    let only_email = web::Data::new(AllowedOtpChannels {
        channels: vec!["email".to_owned()],
        source: "set",
    });
    let app = test::init_service(
        ActixApp::new()
            .app_data(only_email)
            .configure(move |cfg| xchange::bootstrap::configure(cfg, state)),
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
        .set_payload(json!({ "destination": "customer@example.com", "channel": "email" }).to_string())
        .to_request();
    let email_resp = test::call_service(&app, email_req).await;
    assert_eq!(email_resp.status().as_u16(), 200);
    assert_eq!(identity.received("/auth/otp/request").len(), 1);
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
