//! Ticket #4973 (P2): booking display name, booking and queue cancel reasons, assistant message
//! and closure reason take bounded lengths.
//!
//! Behaviour rows (#4973 §1, §3; one test per row: max accepted, max+1 refused with the field):
//! 1. Booking display name: 1..100, field `displayName` (`POST /api/bookings`)
//! 2. Booking cancel reason: 1..200, field `reason` (`POST /api/bookings/{id}/cancel`)
//! 3. Queue ticket cancel reason: 1..200, field `reason` (`POST /api/queue/tickets/{id}/cancel`)
//! 4. Assistant message: 1..500, field `text` (`POST /api/app/assistant/messages`)
//! 5. Closure reason: 1..200, field `reason` (`POST /api/branches/{id}/closures`)

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use bk6_common::*;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

fn accepted(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{what} must be accepted: {}",
        a.text
    );
}

fn refused_on(a: &Answer, field: &str, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST
            && a.body["error"]["code"] == "validationFailed"
            && a.body["error"]["field"] == field,
        "{what}: expected 400 validationFailed on {field:?}; got {} {}",
        a.status,
        a.text
    );
}

fn chars(n: usize) -> String {
    "a".repeat(n)
}

async fn create_open_slot(db: &TestDb) -> Uuid {
    let slot_id = Uuid::new_v4();
    let today = bangkok_today() + chrono::Duration::days(1);
    let start_at = Utc::now() + chrono::Duration::hours(24);
    let end_at = start_at + chrono::Duration::minutes(30);
    seed_slot(
        &db.pool, slot_id, B01, today, start_at, end_at, 10, 0, "OPEN",
    )
    .await;
    slot_id
}

/// Row 1: Booking display name: 1..100, field `displayName`
#[actix_web::test]
async fn test_booking_display_name_bound() {
    let db = TestDb::start().await;
    let slot_id = create_open_slot(&db).await;

    // max+1 refused (101 chars)
    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(json!({
            "branchId": B01,
            "slotId": slot_id,
            "shopSide": "SELL",
            "currency": "USD",
            "quantity": 100.0,
            "phone": "0812345678",
            "displayName": chars(101),
        })),
        Some(&unique("k-create-over")),
    )
    .await;
    refused_on(&res, "displayName", "booking displayName at 101 chars");

    // max accepted (100 chars)
    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/bookings",
        Some(json!({
            "branchId": B01,
            "slotId": slot_id,
            "shopSide": "SELL",
            "currency": "USD",
            "quantity": 100.0,
            "phone": "0812345678",
            "displayName": chars(100),
        })),
        Some(&unique("k-create-at")),
    )
    .await;
    accepted(&res, "booking displayName at 100 chars");

    db.finish().await;
}

/// Row 2: Booking cancel reason: 1..200, field `reason`
#[actix_web::test]
async fn test_booking_cancel_reason_bound() {
    let db = TestDb::start().await;
    let slot_id = Uuid::new_v4();
    let today = bangkok_today() + chrono::Duration::days(1);
    let start_at = Utc::now() + chrono::Duration::hours(24);
    let end_at = start_at + chrono::Duration::minutes(30);
    seed_slot(
        &db.pool, slot_id, B01, today, start_at, end_at, 10, 1, "OPEN",
    )
    .await;
    let booking_id = "BK-test-cancel-bounds";
    seed_booking(
        &db.pool,
        booking_id,
        "B001",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &[0u8; 32],
    )
    .await;

    let uri = format!("/api/bookings/{booking_id}/cancel");

    // max+1 refused (201 chars)
    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({
            "branch": B01,
            "reason": chars(201),
        })),
        Some(&unique("k-cancel-over")),
    )
    .await;
    refused_on(&res, "reason", "booking cancel reason at 201 chars");

    // max accepted (200 chars)
    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({
            "branch": B01,
            "reason": chars(200),
        })),
        Some(&unique("k-cancel-at")),
    )
    .await;
    accepted(&res, "booking cancel reason at 200 chars");

    db.finish().await;
}

/// Row 3: Queue ticket cancel reason: 1..200, field `reason`
#[actix_web::test]
async fn test_queue_cancel_reason_bound() {
    let db = TestDb::start().await;
    let ticket_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, issued_at, idempotency_key) \
         VALUES ($1, $2::uuid, $3, current_date, 'WALK_IN', 1, 'W001', 'WAITING', now(), 'key-q-cancel')",
    )
    .bind(ticket_id)
    .bind(ORG)
    .bind(B01)
    .execute(&db.pool)
    .await
    .expect("insert queue ticket");

    let uri = format!("/api/queue/tickets/{ticket_id}/cancel");

    // max+1 refused (201 chars)
    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({
            "branch": B01,
            "reason": chars(201),
        })),
        Some(&unique("k-q-cancel-over")),
    )
    .await;
    refused_on(&res, "reason", "queue ticket cancel reason at 201 chars");

    // max accepted (200 chars)
    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &uri,
        Some(json!({
            "branch": B01,
            "reason": chars(200),
        })),
        Some(&unique("k-q-cancel-at")),
    )
    .await;
    accepted(&res, "queue ticket cancel reason at 200 chars");

    db.finish().await;
}

/// Row 4: Assistant message: 1..500, field `text`
#[actix_web::test]
async fn test_assistant_message_bound() {
    let db = TestDb::start().await;

    let uri = "/api/app/assistant/messages";

    // max+1 refused (501 chars)
    let res = call(
        &db,
        None,
        Method::POST,
        uri,
        Some(json!({
            "branch": B01,
            "locale": "th",
            "text": chars(501),
        })),
        Some(&unique("k-ask-over")),
    )
    .await;
    refused_on(&res, "text", "assistant text at 501 chars");

    // max accepted (500 chars)
    let res = call(
        &db,
        None,
        Method::POST,
        uri,
        Some(json!({
            "branch": B01,
            "locale": "th",
            "text": chars(500),
        })),
        Some(&unique("k-ask-at")),
    )
    .await;
    accepted(&res, "assistant text at 500 chars");

    db.finish().await;
}

/// Row 5: Closure reason: 1..200, field `reason`
#[actix_web::test]
async fn test_closure_reason_bound() {
    let db = TestDb::start().await;
    let closed_on = bangkok_today() + chrono::Duration::days(5);

    let uri = format!("/api/branches/{B01}/closures");

    // max+1 refused (201 chars)
    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::POST,
        &uri,
        Some(json!({
            "branch": B01,
            "closedOn": closed_on.to_string(),
            "reason": chars(201),
        })),
        Some(&unique("k-closure-over")),
    )
    .await;
    refused_on(&res, "reason", "closure reason at 201 chars");

    // max accepted (200 chars)
    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::POST,
        &uri,
        Some(json!({
            "branch": B01,
            "closedOn": closed_on.to_string(),
            "reason": chars(200),
        })),
        Some(&unique("k-closure-at")),
    )
    .await;
    accepted(&res, "closure reason at 200 chars");

    db.finish().await;
}
