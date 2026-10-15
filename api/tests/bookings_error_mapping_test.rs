// Written before the implementation by the ticket author claude/d7aa64. Do not edit, delete or weaken.

//! The bookings domain's errors reach the wire through one `From<X> for V3Error` per enum
//! (`BookingError`, `CheckInError`, `SlotError`) instead of a per-command catch-all that answers
//! `400 badRequest` with the error's `Display` text.
//!
//! Rows:
//! 1–2. `SlotError::NotFound` answers `404 notFound` (closureAdd / slotRuleSet on an unknown branch).
//! 3.   `StaleVersion` answers `409 staleVersion` (bookingQrReissue on a booking no longer usable).
//! 4.   A store failure answers `500 database`; its body carries no database text (the body half
//!      depends on #2029, which replaces every 5xx message with a generic one).
//! 5.   Keep-pins: existing domain refusals keep their codes and statuses.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use bk6_common::*;

const UNKNOWN_BRANCH: &str = "XC-NO-SUCH-BRANCH";

fn not_found(a: &Answer, what: &str) {
    assert!(
        a.status == StatusCode::NOT_FOUND && a.body["error"]["code"] == "notFound",
        "{what}: expected 404 notFound; got {} {}",
        a.status,
        a.text
    );
}

async fn seed_open_slot(db: &TestDb, capacity: i32, reserved: i32) -> Uuid {
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::minutes(5),
        now + Duration::minutes(35),
        capacity,
        reserved,
        "OPEN",
    )
    .await;
    slot_id
}

// ------------------------------------------------------------ rows 1–2: SlotError::NotFound → 404
//
// The named `branch` must exist (the command gate answers `400 noSuchBranch` otherwise), so the
// unknown branch is the path `{id}` the command writes to; the INSERT … SELECT FROM branches then
// matches no row and the service returns `SlotError::NotFound`.

#[actix_web::test]
async fn closure_add_on_an_unknown_branch_answers_404_not_found() {
    let db = TestDb::start().await;
    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::POST,
        &format!("/api/branches/{UNKNOWN_BRANCH}/closures"),
        Some(json!({ "branch": B01, "closedOn": "2026-12-25", "reason": "Holiday" })),
        Some("k-bkerr-closure-add-404"),
    )
    .await;
    not_found(&res, "closureAdd on an unknown branch");
    db.finish().await;
}

#[actix_web::test]
async fn slot_rule_set_on_an_unknown_branch_answers_404_not_found() {
    let db = TestDb::start().await;
    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::PUT,
        &format!("/api/branches/{UNKNOWN_BRANCH}/booking-slot-rule"),
        Some(json!({ "branch": B01, "slotMinutes": 30, "capacity": 5, "horizonDays": 14 })),
        Some("k-bkerr-rule-404"),
    )
    .await;
    not_found(&res, "slotRuleSet on an unknown branch");
    db.finish().await;
}

/// Keep-pin: removing a date that was never closed is not a `NotFound` (the DELETE matches no
/// row and the command answers 200). The new mapping must not turn it into a 404.
#[actix_web::test]
async fn closure_remove_of_a_date_never_closed_stays_200() {
    let db = TestDb::start().await;
    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::POST,
        &format!("/api/branches/{B01}/closures/2026-12-24/remove"),
        Some(json!({ "branch": B01 })),
        Some("k-bkerr-closure-rm-none"),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "closureRemove of a date never closed: expected 200; got {} {}",
        res.status,
        res.text
    );
    db.finish().await;
}

/// Keep-pin: the slot commands already answer 404 for an unknown slot id.
#[actix_web::test]
async fn slot_commands_on_an_unknown_slot_answer_404_not_found() {
    let db = TestDb::start().await;
    let missing = Uuid::new_v4();
    let cases = [
        (
            "slotCapacitySet",
            Method::PUT,
            format!("/api/booking-slots/{missing}/capacity"),
            json!({ "branch": B01, "capacity": 9 }),
        ),
        (
            "slotClose",
            Method::POST,
            format!("/api/booking-slots/{missing}/close"),
            json!({ "branch": B01 }),
        ),
        (
            "slotOpen",
            Method::POST,
            format!("/api/booking-slots/{missing}/open"),
            json!({ "branch": B01 }),
        ),
    ];
    for (name, method, uri, body) in cases {
        let key = format!("k-bkerr-404-{}", name.to_lowercase());
        let res = call(&db, Some("ADMIN@*"), method, &uri, Some(body), Some(&key)).await;
        not_found(&res, &format!("{name} on an unknown slot"));
    }
    db.finish().await;
}

// ------------------------------------------------------------ row 3: StaleVersion → 409

#[actix_web::test]
async fn qr_reissue_on_a_cancelled_booking_answers_409_stale_version() {
    let db = TestDb::start().await;
    let slot_id = seed_open_slot(&db, 5, 0).await;
    let booking_id = "BK-bkerr-qr-stale";
    seed_booking(
        &db.pool,
        booking_id,
        "B261016-9001",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CANCELLED",
        b"qr-hash",
    )
    .await;

    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/bookings/{booking_id}/qr"),
        Some(json!({ "branch": B01 })),
        Some("k-bkerr-qr-stale"),
    )
    .await;
    assert!(
        res.status == StatusCode::CONFLICT && res.body["error"]["code"] == "staleVersion",
        "bookingQrReissue on a CANCELLED booking: expected 409 staleVersion; got {} {}",
        res.status,
        res.text
    );
    db.finish().await;
}

// ------------------------------------------------------------ row 4: Store → 500 database

/// `requested_quantity numeric(20,4)` holds at most 16 integer digits; 1e16 overflows it
/// (migrations/20261205131000_booking_slots.sql:92) and nothing bounds it before the INSERT.
async fn create_with_overflowing_quantity(db: &TestDb, key: &str) -> Answer {
    let slot_id = seed_open_slot(db, 5, 0).await;
    call(
        db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(json!({
            "branch": B01,
            "branchId": B01,
            "slotId": slot_id,
            "shopSide": "BUY",
            "currency": "USD",
            "quantity": "10000000000000000",
        })),
        Some(key),
    )
    .await
}

#[actix_web::test]
async fn booking_create_store_failure_answers_500_database() {
    let db = TestDb::start().await;
    let res = create_with_overflowing_quantity(&db, "k-bkerr-overflow-1").await;
    assert!(
        res.status == StatusCode::INTERNAL_SERVER_ERROR && res.body["error"]["code"] == "database",
        "bookingCreate with a quantity that overflows numeric(20,4): expected 500 database; got {} {}",
        res.status,
        res.text
    );
    db.finish().await;
}

/// Depends on #2029 (a 5xx carries a generic message, the detail goes to the server log).
/// Until #2029 merges, a `V3Error` built from `e.to_string()` still carries the sqlx text.
#[actix_web::test]
async fn booking_create_store_failure_body_carries_no_database_text() {
    let db = TestDb::start().await;
    let res = create_with_overflowing_quantity(&db, "k-bkerr-overflow-2").await;
    assert_eq!(
        res.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "expected 500; got {} {}",
        res.status,
        res.text
    );
    let lower = res.text.to_lowercase();
    for leak in [
        "database error",
        "numeric",
        "overflow",
        "relation",
        "constraint",
    ] {
        assert!(
            !lower.contains(leak),
            "the 500 body must not carry database text `{leak}`; got {}",
            res.text
        );
    }
    db.finish().await;
}

// ------------------------------------------------------------ row 5: keep-pins

/// `BookingError::CapacityFull` keeps `400 capacityFull`.
#[actix_web::test]
async fn keep_capacity_full_400() {
    let db = TestDb::start().await;
    let slot_id = seed_open_slot(&db, 1, 1).await;
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(json!({
            "branch": B01,
            "branchId": B01,
            "slotId": slot_id,
            "shopSide": "BUY",
            "currency": "USD",
            "quantity": 100.0,
        })),
        Some("k-bkerr-full"),
    )
    .await;
    assert!(
        res.status == StatusCode::BAD_REQUEST && res.body["error"]["code"] == "capacityFull",
        "create on a full slot: expected 400 capacityFull; got {} {}",
        res.status,
        res.text
    );
    db.finish().await;
}

/// `CheckInError::QrMismatch` keeps `400 qrMismatch`.
#[actix_web::test]
async fn keep_qr_mismatch_400() {
    let db = TestDb::start().await;
    let slot_id = seed_open_slot(&db, 5, 1).await;
    let booking_id = "BK-bkerr-qr-mismatch";
    seed_booking(
        &db.pool,
        booking_id,
        "B261016-9002",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        b"not-the-hash-of-the-token",
    )
    .await;
    let res = call(
        &db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/bookings/{booking_id}/check-in"),
        Some(json!({ "qrToken": "wrong-qr-token" })),
        Some("k-bkerr-qr-mismatch"),
    )
    .await;
    assert!(
        res.status == StatusCode::BAD_REQUEST && res.body["error"]["code"] == "qrMismatch",
        "check-in with a mismatched QR: expected 400 qrMismatch; got {} {}",
        res.status,
        res.text
    );
    db.finish().await;
}

/// `SlotError::CapacityBelowReserved` keeps `400 capacityBelowReserved`.
#[actix_web::test]
async fn keep_capacity_below_reserved_400() {
    let db = TestDb::start().await;
    let slot_id = seed_open_slot(&db, 5, 3).await;
    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::PUT,
        &format!("/api/booking-slots/{slot_id}/capacity"),
        Some(json!({ "branch": B01, "capacity": 2 })),
        Some("k-bkerr-below"),
    )
    .await;
    assert!(
        res.status == StatusCode::BAD_REQUEST
            && res.body["error"]["code"] == "capacityBelowReserved",
        "capacity below reserved: expected 400 capacityBelowReserved; got {} {}",
        res.status,
        res.text
    );
    db.finish().await;
}
