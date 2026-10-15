// Written before the implementation by the test writer gemini/pool-5 (controller claude/e92db6). Do not edit, delete or weaken.

//! Specification tests for BK-6 (#1388, docs/design/blueprint/booking-slots.md §4.4, §9.1):
//! bookingCheckIn (`POST /api/bookings/{id}/check-in`) and kioskBookingCheckIn (`POST /api/kiosk/bookings/check-in`),
//! permissions, validation rules (controller ruling 3), and ticket issuance.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

#[actix_web::test]
async fn check_in_route_refuses_without_bookings_desk_permission() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-checkin-perm";
    let qr_token = "valid-qr-token-perm";
    let qr_hash = Sha256::digest(qr_token.as_bytes());
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0001",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/check-in");
    let payload = json!({
        "identityChecked": true
    });

    // 1. Unauthenticated -> 401
    let res_anon = call(
        &db,
        None,
        Method::POST,
        &uri,
        Some(payload.clone()),
        Some("k-ci-anon"),
    )
    .await;
    assert_eq!(res_anon.status, StatusCode::UNAUTHORIZED);

    // 2. Customer -> 400 roleNotAllowed
    let res_cust = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &uri,
        Some(payload),
        Some("k-ci-cust"),
    )
    .await;
    policy_refused(&res_cust, "check-in by CUSTOMER");

    db.finish().await;
}

#[actix_web::test]
async fn check_in_refuses_both_qr_token_and_identity_checked() {
    // Controller ruling 3 on #1388: bookingCheckIn with both qrToken and identityChecked -> validationFailed
    let db = TestDb::start().await;
    let booking_id = "BK-test-both-options";
    let uri = format!("/api/bookings/{booking_id}/check-in");
    let payload = json!({
        "qrToken": "some-token",
        "identityChecked": true
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(payload),
        Some("k-ci-both"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::BAD_REQUEST,
        "both qrToken and identityChecked must be refused 400 validationFailed; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(res.body["error"]["code"], "validationFailed");

    db.finish().await;
}

#[actix_web::test]
async fn check_in_refuses_neither_qr_token_nor_identity_checked() {
    // Controller ruling 3 on #1388: bookingCheckIn with neither qrToken nor identityChecked -> validationFailed
    let db = TestDb::start().await;
    let booking_id = "BK-test-neither-option";
    let uri = format!("/api/bookings/{booking_id}/check-in");
    let payload = json!({});

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(payload),
        Some("k-ci-neither"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::BAD_REQUEST,
        "neither qrToken nor identityChecked must be refused 400 validationFailed; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(res.body["error"]["code"], "validationFailed");

    db.finish().await;
}

#[actix_web::test]
async fn check_in_by_valid_qr_token_issues_booking_ticket() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-checkin-qr-ok";
    let qr_token = "valid-qr-token-12345";
    let qr_hash = Sha256::digest(qr_token.as_bytes());
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0002",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/check-in");
    let payload = json!({
        "qrToken": qr_token
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(payload),
        Some("k-ci-qr-1"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "check-in with valid QR token should succeed 200 OK; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CHECKED_IN"));

    // Verify booking ticket was issued
    let ticket_no = res.body["ticketNumber"]
        .as_str()
        .or_else(|| res.body["data"]["ticketNumber"].as_str());
    assert!(
        ticket_no.is_some(),
        "response must carry issued ticket number"
    );

    db.finish().await;
}

#[actix_web::test]
async fn check_in_by_identity_checked_issues_booking_ticket() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-checkin-id-ok";
    let qr_hash = Sha256::digest(b"dummy-qr-id");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0003",
        CUSTOMER_BOB_CUST_ID,
        B01,
        slot_id,
        "DESK",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/check-in");
    let payload = json!({
        "identityChecked": true
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(payload),
        Some("k-ci-id-1"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "check-in with identityChecked should succeed 200 OK; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CHECKED_IN"));

    db.finish().await;
}

#[actix_web::test]
async fn check_in_refuses_mismatched_qr_token() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-checkin-mismatch";
    let qr_hash = Sha256::digest(b"correct-qr-token");
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0004",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/check-in");
    let payload = json!({
        "qrToken": "wrong-qr-token"
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(payload),
        Some("k-ci-wrong"),
    )
    .await;
    refused_with(&res, "qrMismatch", "check-in with mismatched QR");

    db.finish().await;
}

#[actix_web::test]
async fn check_in_outside_window_refuses_check_in_window() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    // Slot is tomorrow
    let tomorrow = bangkok_today() + Duration::days(1);
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        tomorrow,
        now + Duration::days(1),
        now + Duration::days(1) + Duration::minutes(30),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-checkin-future";
    let qr_token = "qr-future";
    let qr_hash = Sha256::digest(qr_token.as_bytes());
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0005",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/check-in");
    let payload = json!({
        "qrToken": qr_token
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(payload),
        Some("k-ci-future"),
    )
    .await;
    refused_with(&res, "checkInWindow", "check-in outside window");

    db.finish().await;
}

#[actix_web::test]
async fn kiosk_check_in_by_valid_qr_token_issues_ticket() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-kiosk-checkin";
    let qr_token = "kiosk-qr-token-999";
    let qr_hash = Sha256::digest(qr_token.as_bytes());
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0006",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = "/api/kiosk/bookings/check-in";
    let payload = json!({
        "branch": B01,
        "qrToken": qr_token
    });

    let res = call(
        &db,
        None,
        Method::POST,
        uri,
        Some(payload),
        Some("k-kiosk-1"),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "kiosk check-in with valid QR should succeed 200 OK; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CHECKED_IN"));

    db.finish().await;
}

#[actix_web::test]
async fn check_in_idempotency_replay_and_conflict() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-checkin-idemp";
    let qr_token = "qr-token-idemp";
    let qr_hash = Sha256::digest(qr_token.as_bytes());
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0007",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/check-in");
    let key = "key-ci-replay-1";
    let payload = json!({
        "qrToken": qr_token
    });

    let res1 = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(payload.clone()),
        Some(key),
    )
    .await;
    assert_eq!(res1.status, StatusCode::OK);

    // Replay
    let res2 = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(payload),
        Some(key),
    )
    .await;
    assert_eq!(res2.status, StatusCode::OK);

    // Conflict with different payload
    let res3 = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({ "identityChecked": true })),
        Some(key),
    )
    .await;
    assert_eq!(res3.status, StatusCode::CONFLICT);
    assert_eq!(res3.body["error"]["code"], "idempotencyConflict");

    db.finish().await;
}

#[actix_web::test]
async fn kiosk_check_in_idempotency_replay_and_conflict() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        5,
        1,
        "OPEN",
    )
    .await;

    let booking_id = "BK-test-kiosk-idemp";
    let qr_token = "kiosk-idemp-token-1";
    let qr_hash = Sha256::digest(qr_token.as_bytes());
    seed_booking(
        &db.pool,
        booking_id,
        "B261014-0008",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let uri = "/api/kiosk/bookings/check-in";
    let key = "key-kiosk-ci-1";
    let payload = json!({
        "branch": B01,
        "qrToken": qr_token
    });

    let res1 = call(
        &db,
        None,
        Method::POST,
        uri,
        Some(payload.clone()),
        Some(key),
    )
    .await;
    assert_eq!(res1.status, StatusCode::OK);

    // Replay
    let res2 = call(&db, None, Method::POST, uri, Some(payload), Some(key)).await;
    assert_eq!(res2.status, StatusCode::OK);

    // Conflict with different payload
    let res3 = call(
        &db,
        None,
        Method::POST,
        uri,
        Some(json!({ "branch": B01, "qrToken": "different-token" })),
        Some(key),
    )
    .await;
    assert_eq!(res3.status, StatusCode::CONFLICT);
    assert_eq!(res3.body["error"]["code"], "idempotencyConflict");

    db.finish().await;
}
