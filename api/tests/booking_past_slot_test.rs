// Written before the implementation by the ticket author's lane (claude/28619d18, for claude/7ca80c).
// Do not edit, delete or weaken these tests; add your own in another file.

//! Up-front tests for #4899: a slot that has already started is neither offered nor refused as full.
//!
//! Today the `booking_slots` view lists every OPEN slot of the date with no time filter
//! (`v3/views/booking_slots.rs:56-60`), and create / reschedule refuse a started slot as
//! `BookingError::SlotUnavailable` (`xc-booking/src/application/booking_service.rs:200`, `:779-785`),
//! which both handlers answer as `capacityFull` (`v3/commands/bookings/create.rs:213-218`,
//! `reschedule.rs:142-146`).
//!
//! Status: #4899 §2.2 gives `slotUnavailable` the status `capacityFull` has. That status is `400`
//! (`create.rs:207-212`), and `bookings_error_mapping_test.rs::keep_capacity_full_400` pins it, so
//! S2 and S4 expect `400 slotUnavailable` and S3 expects `400 capacityFull`.
//!
//! Behaviour rows (#4899 §3):
//! | # | Case | Expect |
//! |---|---|---|
//! | S1 | view for today, one slot started 1 min ago, one in 1 h | only the later slot |
//! | S2 | create on a started slot | 400 `slotUnavailable`, no booking written |
//! | S3 | create on a full future slot | 400 `capacityFull` (as today) |
//! | S4 | reschedule onto a started slot | 400 `slotUnavailable`, the booking stays on its slot |
//! | S5 | create with `quantity: "100.5"` (string) and `quantity: 100.5` (number) | both accepted (as today on the wire) |

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

/// Seeds an OPEN slot on today's Bangkok date at `B01` that starts `from_now` from now and runs 30 minutes.
async fn slot_at(db: &TestDb, from_now: Duration, capacity: i32, reserved: i32) -> Uuid {
    let slot_id = Uuid::new_v4();
    let start = Utc::now() + from_now;
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        start,
        start + Duration::minutes(30),
        capacity,
        reserved,
        "OPEN",
    )
    .await;
    slot_id
}

async fn create(db: &TestDb, slot_id: Uuid, quantity: Value, key: &str) -> Answer {
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
            "quantity": quantity,
        })),
        Some(key),
    )
    .await
}

async fn bookings_on(db: &TestDb, slot_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM bookings WHERE slot_id = $1")
        .bind(slot_id)
        .fetch_one(&db.pool)
        .await
        .expect("count bookings on the slot")
}

fn answered(a: &Answer, status: StatusCode, code: &str, what: &str) {
    assert!(
        a.status == status && a.body["error"]["code"] == code,
        "{what}: expected {status} {code}; got {} {}",
        a.status,
        a.text
    );
}

#[actix_web::test]
async fn s1_view_for_today_lists_only_slots_not_yet_started() {
    let db = TestDb::start().await;
    let started = slot_at(&db, Duration::minutes(-1), 5, 0).await;
    let later = slot_at(&db, Duration::hours(1), 5, 0).await;

    let uri = format!(
        "/api/v3/views/booking_slots?branch={B01}&date={}",
        bangkok_today()
    );
    let res = call(&db, None, Method::GET, &uri, None, None).await;

    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    let ids: Vec<&str> = res.body["data"]["slots"]
        .as_array()
        .unwrap_or_else(|| panic!("expected data.slots array in {}", res.text))
        .iter()
        .map(|s| s["id"].as_str().expect("slot id"))
        .collect();
    assert_eq!(
        ids,
        vec![later.to_string().as_str()],
        "the slot that started at now-1min ({started}) must not be offered: {}",
        res.text
    );

    db.finish().await;
}

#[actix_web::test]
async fn s2_create_on_a_started_slot_answers_slot_unavailable() {
    let db = TestDb::start().await;
    let started = slot_at(&db, Duration::minutes(-1), 5, 0).await;

    let res = create(&db, started, json!("100"), "k-past-s2").await;

    answered(
        &res,
        StatusCode::BAD_REQUEST,
        "slotUnavailable",
        "create on a started slot",
    );
    assert_eq!(bookings_on(&db, started).await, 0, "nothing is booked");

    db.finish().await;
}

#[actix_web::test]
async fn s3_create_on_a_full_future_slot_answers_capacity_full_as_today() {
    let db = TestDb::start().await;
    let full = slot_at(&db, Duration::hours(1), 1, 1).await;

    let res = create(&db, full, json!("100"), "k-past-s3").await;

    answered(
        &res,
        StatusCode::BAD_REQUEST,
        "capacityFull",
        "create on a full future slot",
    );
    assert_eq!(bookings_on(&db, full).await, 0, "nothing is booked");

    db.finish().await;
}

#[actix_web::test]
async fn s4_reschedule_onto_a_started_slot_answers_slot_unavailable() {
    let db = TestDb::start().await;
    // The booked slot is 4 h away, outside the customer's 60-minute cutoff.
    let booked = slot_at(&db, Duration::hours(4), 5, 1).await;
    let started = slot_at(&db, Duration::minutes(-1), 5, 0).await;
    let booking_id = "BK-test-past-slot-resched";
    let qr_hash = Sha256::digest(b"dummy-qr-past-slot-resched");
    seed_booking(
        &db.pool,
        booking_id,
        "B261023-0041",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        booked,
        "APP",
        "CONFIRMED",
        &qr_hash,
    )
    .await;

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &format!("/api/bookings/{booking_id}/reschedule"),
        Some(json!({ "branch": B01, "slotId": started })),
        Some("k-past-s4"),
    )
    .await;

    answered(
        &res,
        StatusCode::BAD_REQUEST,
        "slotUnavailable",
        "reschedule onto a started slot",
    );
    let slot: Uuid = sqlx::query_scalar("SELECT slot_id FROM bookings WHERE id = $1")
        .bind(booking_id)
        .fetch_one(&db.pool)
        .await
        .expect("read the booking");
    assert_eq!(slot, booked, "the booking stays on its slot");

    db.finish().await;
}

#[actix_web::test]
async fn s5_create_accepts_quantity_as_string_and_as_number() {
    let db = TestDb::start().await;

    for (quantity, key) in [
        (json!("100.5"), "k-past-s5-string"),
        (json!(100.5), "k-past-s5-number"),
    ] {
        let slot = slot_at(&db, Duration::hours(1), 5, 0).await;
        let res = create(&db, slot, quantity.clone(), key).await;
        assert!(
            res.status.is_success(),
            "create with quantity {quantity} must be accepted; got {} {}",
            res.status,
            res.text
        );
        assert_eq!(
            bookings_on(&db, slot).await,
            1,
            "quantity {quantity} booked"
        );
    }

    db.finish().await;
}
