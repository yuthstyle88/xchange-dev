//! Ticket #2699: Desk booking with an unreadable phone answers 400 validationFailed
//! on phone, while valid phone, existing customer, and missing customerId are handled per behaviour table.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use bk6_common::*;

/// Helper to create and seed an open slot for testing.
async fn create_open_slot(db: &TestDb) -> Uuid {
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(1),
        now + Duration::hours(2),
        5,
        0,
        "OPEN",
    )
    .await;
    slot_id
}

/// P1: contact: {displayName: "Walk-in", phone: "bad-phone"}
/// -> 400 validationFailed, field phone; no customer, identity or booking row (main: 400 badRequest, no field)
#[actix_web::test]
async fn p1_desk_booking_with_invalid_phone_refused_validation_failed_on_phone() {
    let db = TestDb::start().await;
    let slot_id = create_open_slot(&db).await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "SELL",
        "currency": "USD",
        "quantity": 250.0,
        "displayName": "Walk-in",
        "phone": "bad-phone",
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-bad-phone"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::BAD_REQUEST,
        "expected 400 BAD_REQUEST for invalid phone; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(
        res.body["error"]["code"], "validationFailed",
        "expected code validationFailed; got {}",
        res.text
    );
    assert_eq!(
        res.body["error"]["field"], "phone",
        "expected field phone; got {}",
        res.text
    );

    // Verify no customer, identity or booking rows were written
    let customer_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    // TestDb seeds exactly 2 customers (Alice, Bob)
    assert_eq!(customer_count, 2, "no new customer row must be created");

    let identity_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customer_identities")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        identity_count, 0,
        "no customer_identities row must be created"
    );

    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(booking_count, 0, "no booking row must be created");

    db.finish().await;
}

/// P2: contact with +66812345678 -> 200, booking CONFIRMED (kept)
#[actix_web::test]
async fn p2_desk_booking_with_valid_phone_succeeds_confirmed() {
    let db = TestDb::start().await;
    let slot_id = create_open_slot(&db).await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "SELL",
        "currency": "USD",
        "quantity": 250.0,
        "displayName": "Walk-in",
        "phone": "+66812345678",
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-valid-phone"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "expected 200 OK for valid phone; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CONFIRMED"));

    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(booking_count, 1, "booking row must be created");

    db.finish().await;
}

/// P2 edge case: "081-234-5678" is readable (normalised to +66812345678), so it behaves as P2.
#[actix_web::test]
async fn p2_edge_desk_booking_with_local_formatted_phone_succeeds_confirmed() {
    let db = TestDb::start().await;
    let slot_id = create_open_slot(&db).await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "SELL",
        "currency": "USD",
        "quantity": 250.0,
        "displayName": "Walk-in Local",
        "phone": "081-234-5678",
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-local-phone"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "expected 200 OK for local formatted phone; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CONFIRMED"));

    db.finish().await;
}

/// P3: customerId of an existing customer -> unchanged (kept, 200 CONFIRMED)
#[actix_web::test]
async fn p3_desk_booking_with_existing_customer_id_succeeds_confirmed() {
    let db = TestDb::start().await;
    let slot_id = create_open_slot(&db).await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "SELL",
        "currency": "USD",
        "quantity": 250.0,
        "customerId": CUSTOMER_ALICE_CUST_ID,
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-existing-cust"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::OK,
        "expected 200 OK for existing customerId; got {} {}",
        res.status,
        res.text
    );

    let status = res.body["status"]
        .as_str()
        .or_else(|| res.body["data"]["status"].as_str());
    assert_eq!(status, Some("CONFIRMED"));

    db.finish().await;
}

/// P4: neither customerId nor contact -> 400 validationFailed on customerId (kept)
#[actix_web::test]
async fn p4_desk_booking_with_neither_customer_id_nor_contact_refused_validation_failed() {
    let db = TestDb::start().await;
    let slot_id = create_open_slot(&db).await;

    let payload = json!({
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "SELL",
        "currency": "USD",
        "quantity": 250.0,
    });

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-desk-no-ident"),
    )
    .await;

    assert_eq!(
        res.status,
        StatusCode::BAD_REQUEST,
        "expected 400 BAD_REQUEST for missing customerId/contact; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(
        res.body["error"]["code"], "validationFailed",
        "expected code validationFailed; got {}",
        res.text
    );
    assert_eq!(
        res.body["error"]["field"], "customerId",
        "expected field customerId; got {}",
        res.text
    );

    db.finish().await;
}
