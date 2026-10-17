//! #2007: pin the branch check (`if branch_id != branch.as_str() { return Err(out_of_scope()); }`)
//! in `bookingCancel`, `bookingReschedule`, `bookingQrReissue`, and `bookingCheckIn`
//! (`docs/design/blueprint/booking-slots.md` §4.5; `bookingConfirm` is pinned at
//! `api/tests/bk6_shared_runner_test.rs:208`).

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::Method;
use chrono::{Duration, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

const B02: &str = "XC-B02";

async fn seed_branch_b02(db: &TestDb) {
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id, booking_enabled) \
         VALUES ($1, 'Branch 2', $2::uuid, true)",
    )
    .bind(B02)
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("a second branch of the same organisation");

    sqlx::query(
        "INSERT INTO branch_service_status (branch_id, organization_id, queue_open, active_counters, queue_version) \
         VALUES ($1, $2::uuid, true, 1, 1) \
         ON CONFLICT (branch_id) DO UPDATE SET queue_open = true, active_counters = 1, queue_version = 1",
    )
    .bind(B02)
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("open queue on B02");
}

async fn status_of(db: &TestDb, booking_id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM bookings WHERE id = $1")
        .bind(booking_id)
        .fetch_one(&db.pool)
        .await
        .expect("the booking's status")
}

async fn slot_of(db: &TestDb, booking_id: &str) -> Uuid {
    sqlx::query_scalar("SELECT slot_id FROM bookings WHERE id = $1")
        .bind(booking_id)
        .fetch_one(&db.pool)
        .await
        .expect("the booking's slot_id")
}

async fn qr_hash_of(db: &TestDb, booking_id: &str) -> Vec<u8> {
    sqlx::query_scalar("SELECT qr_token_hash FROM bookings WHERE id = $1")
        .bind(booking_id)
        .fetch_one(&db.pool)
        .await
        .expect("the booking's qr_token_hash")
}

async fn alice_booking(
    db: &TestDb,
    booking_id: &str,
    booking_no: &str,
    status: &str,
    qr: &str,
) -> Uuid {
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        now + Duration::hours(2),
        now + Duration::hours(3),
        5,
        1,
        "OPEN",
    )
    .await;
    seed_booking(
        &db.pool,
        booking_id,
        booking_no,
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        status,
        &Sha256::digest(qr.as_bytes()),
    )
    .await;
    slot_id
}

#[actix_web::test]
async fn booking_cancel_that_names_another_branch_is_out_of_scope() {
    let db = TestDb::start().await;
    seed_branch_b02(&db).await;
    let id = "BK-scope-cancel";
    alice_booking(&db, id, "B261015-0201", "CONFIRMED", "qr-cancel-scope").await;

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &format!("/api/bookings/{id}/cancel"),
        Some(json!({ "branch": B02 })),
        Some("k-scope-cancel"),
    )
    .await;
    refused_with(
        &res,
        "outOfScope",
        "a B01 booking cancelled while naming B02",
    );
    assert_eq!(status_of(&db, id).await, "CONFIRMED");

    db.finish().await;
}

#[actix_web::test]
async fn booking_reschedule_that_names_another_branch_is_out_of_scope() {
    let db = TestDb::start().await;
    seed_branch_b02(&db).await;
    let id = "BK-scope-reschedule";
    let orig_slot =
        alice_booking(&db, id, "B261015-0202", "CONFIRMED", "qr-reschedule-scope").await;

    let new_slot = Uuid::new_v4();
    let now = Utc::now();
    seed_slot(
        &db.pool,
        new_slot,
        B01,
        bangkok_today(),
        now + Duration::hours(4),
        now + Duration::hours(5),
        5,
        0,
        "OPEN",
    )
    .await;

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &format!("/api/bookings/{id}/reschedule"),
        Some(json!({ "branch": B02, "slotId": new_slot })),
        Some("k-scope-reschedule"),
    )
    .await;
    refused_with(
        &res,
        "outOfScope",
        "a B01 booking rescheduled while naming B02",
    );
    assert_eq!(slot_of(&db, id).await, orig_slot);

    db.finish().await;
}

#[actix_web::test]
async fn booking_qr_reissue_that_names_another_branch_is_out_of_scope() {
    let db = TestDb::start().await;
    seed_branch_b02(&db).await;
    let id = Uuid::new_v4().to_string();
    let id = id.as_str();
    let qr = "qr-reissue-scope";
    alice_booking(&db, id, "B261015-0203", "CONFIRMED", qr).await;
    let orig_hash = qr_hash_of(&db, id).await;

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &format!("/api/bookings/{id}/qr"),
        Some(json!({ "branch": B02 })),
        Some("k-scope-qr"),
    )
    .await;
    refused_with(
        &res,
        "outOfScope",
        "a B01 booking QR reissued while naming B02",
    );
    assert_eq!(qr_hash_of(&db, id).await, orig_hash);

    db.finish().await;
}

#[actix_web::test]
async fn booking_check_in_that_names_another_branch_is_out_of_scope() {
    let db = TestDb::start().await;
    seed_branch_b02(&db).await;
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
    let id = "BK-scope-checkin";
    seed_booking(
        &db.pool,
        id,
        "B261015-0204",
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &Sha256::digest(b"qr-checkin-scope"),
    )
    .await;

    let res = call(
        &db,
        Some("TELLER@XC-B02"),
        Method::POST,
        &format!("/api/bookings/{id}/check-in"),
        Some(json!({ "branch": B02, "identityChecked": true })),
        Some("k-scope-checkin"),
    )
    .await;
    refused_with(
        &res,
        "outOfScope",
        "a B01 booking checked in by B02 staff while naming B02",
    );
    assert_eq!(status_of(&db, id).await, "CONFIRMED");

    db.finish().await;
}
