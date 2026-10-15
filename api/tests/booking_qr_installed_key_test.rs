//! Regression tests for #1850: booking QR tokens must be derived from the installed PII keys
//! (`xc_customer::get_pii_keys()`), never from a hardcoded test key in source.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;
use xc_booking::domain::booking::qr_token::QrToken;
use xc_kernel::pii::PiiKeys;

use bk6_common::*;

const INSTALLED_KEY_SPEC: &str = "prod-1:2iq82iq82iq82iq82iq82iq82iq82iq82iq82iq82io=";
const FIXED_TEST_KEY_SPEC: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn ensure_non_test_pii_keys_installed() -> Arc<PiiKeys> {
    let keys = Arc::new(PiiKeys::parse(INSTALLED_KEY_SPEC).expect("parse installed PII key"));
    xc_customer::install_pii_keys(keys.clone())
        .expect("install non-test PII key once for this test binary");
    let current = xc_customer::get_pii_keys().expect("installed PII keys available");
    assert_eq!(
        current.current_id(),
        "prod-1",
        "process must hold the non-test PII key, not the test fixture key"
    );
    current
}

fn fixed_test_pii_keys() -> PiiKeys {
    PiiKeys::parse(FIXED_TEST_KEY_SPEC).expect("parse fixed test PII key")
}

#[actix_web::test]
async fn booking_qr_token_verifies_under_installed_key_and_not_fixed_test_key() {
    let installed_keys = ensure_non_test_pii_keys_installed();
    let fixed_test_keys = fixed_test_pii_keys();

    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(10),
        now + Duration::minutes(40),
        5,
        0,
        "OPEN",
    )
    .await;

    let create_payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": "USD",
        "quantity": 100.0,
    });

    let create_res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(create_payload),
        Some("k-1850-create-1"),
    )
    .await;
    assert_eq!(
        create_res.status,
        StatusCode::OK,
        "expected 200 OK creating booking; got {} {}",
        create_res.status,
        create_res.text
    );

    let booking_id = create_res.body["id"]
        .as_str()
        .or_else(|| create_res.body["data"]["id"].as_str())
        .expect("booking id in create response");
    let qr_token = create_res.body["qrToken"]
        .as_str()
        .or_else(|| create_res.body["data"]["qrToken"].as_str())
        .expect("qrToken in create response");

    let booking_uuid = Uuid::parse_str(
        booking_id
            .strip_prefix("BK-")
            .expect("booking id has BK- prefix"),
    )
    .expect("booking id has valid UUID");

    let expected_installed_token =
        QrToken::derive(&QrToken::key_from_pii(&installed_keys), &booking_uuid, 1);
    let fixed_test_token =
        QrToken::derive(&QrToken::key_from_pii(&fixed_test_keys), &booking_uuid, 1);

    // (b) The issued token must NOT verify under the fixed test key.
    assert_ne!(
        qr_token, fixed_test_token,
        "(b) booking QR token was signed with the fixed test key instead of the installed key"
    );

    // (a) The issued token must verify under QrToken::key_from_pii(<installed key>).
    assert_eq!(
        qr_token, expected_installed_token,
        "(a) booking QR token must verify under QrToken::key_from_pii(<installed key>)"
    );

    // (c) Check-in through the route accepts the token.
    let check_in_uri = format!("/api/bookings/{booking_id}/check-in");
    let check_in_res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &check_in_uri,
        Some(json!({ "qrToken": qr_token })),
        Some("k-1850-checkin-1"),
    )
    .await;
    assert_eq!(
        check_in_res.status,
        StatusCode::OK,
        "(c) check-in must accept the QR token signed with the installed key; got {} {}",
        check_in_res.status,
        check_in_res.text
    );
    let status = check_in_res.body["status"]
        .as_str()
        .or_else(|| check_in_res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CHECKED_IN"));

    db.finish().await;
}

#[actix_web::test]
async fn check_in_refuses_token_minted_under_fixed_test_key_and_accepts_installed_key() {
    let installed_keys = ensure_non_test_pii_keys_installed();
    let fixed_test_keys = fixed_test_pii_keys();

    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(10),
        now + Duration::minutes(40),
        5,
        0,
        "OPEN",
    )
    .await;

    let create_payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "SELL",
        "currency": "USD",
        "quantity": 200.0,
    });

    let create_res = call(
        &db,
        Some("CUSTOMER@customer-bob"),
        Method::POST,
        "/api/bookings",
        Some(create_payload),
        Some("k-1850-create-2"),
    )
    .await;
    assert_eq!(create_res.status, StatusCode::OK);

    let booking_id = create_res.body["id"]
        .as_str()
        .or_else(|| create_res.body["data"]["id"].as_str())
        .expect("booking id");
    let booking_uuid = Uuid::parse_str(
        booking_id
            .strip_prefix("BK-")
            .expect("booking id has BK- prefix"),
    )
    .expect("booking id has valid UUID");

    let forged_test_token =
        QrToken::derive(&QrToken::key_from_pii(&fixed_test_keys), &booking_uuid, 1);
    let installed_key_token =
        QrToken::derive(&QrToken::key_from_pii(&installed_keys), &booking_uuid, 1);

    let check_in_uri = format!("/api/bookings/{booking_id}/check-in");

    // A token minted under the fixed test key must be refused at check-in.
    let forged_res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &check_in_uri,
        Some(json!({ "qrToken": forged_test_token })),
        Some("k-1850-checkin-forged"),
    )
    .await;
    refused_with(
        &forged_res,
        "qrMismatch",
        "(c) token minted under fixed test key must be refused at check-in",
    );

    // The token derived from the installed key must be accepted at check-in.
    let valid_res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &check_in_uri,
        Some(json!({ "qrToken": installed_key_token })),
        Some("k-1850-checkin-valid"),
    )
    .await;
    assert_eq!(
        valid_res.status,
        StatusCode::OK,
        "(c) token derived from installed key must be accepted at check-in; got {} {}",
        valid_res.status,
        valid_res.text
    );

    db.finish().await;
}
