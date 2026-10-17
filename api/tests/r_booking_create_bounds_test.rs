// Written before the implementation by claude/studio-claude-2 (BE controller claude/56a74fcd). Do not edit, delete or weaken.

//! R-ticket of #2054 for `bookingCreate` (`POST /api/bookings`, `commands/bookings/create.rs`), design
//! `docs/design/blueprint/bounded-inputs.md` B-3, B-4, B-7.
//!
//! - `quantity` is bounded by `bookings.requested_quantity numeric(20,4) CHECK (> 0)`
//!   (`migrations/20261205131000_booking_slots.sql:92`): positive, at most 4 decimals, below 10^16.
//! - `currency` must name a `currencies` row (`:91`, `REFERENCES currencies (code)`), checked before the write.
//! - A refusal is **400 `validationFailed`** naming the field, and nothing is written.
//! - During the change a JSON number is still accepted for `quantity` (the web client sends one,
//!   `web/src/screens/customer/book/Book.tsx:359`); it is bounded the same way.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use bk6_common::*;

const CUSTOMER: &str = "CUSTOMER@customer-alice";

async fn open_slot(db: &TestDb) -> Uuid {
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

fn body(slot_id: Uuid, currency: &str, quantity: Value) -> Value {
    json!({
        "branch": B01,
        "branchId": B01,
        "slotId": slot_id,
        "shopSide": "BUY",
        "currency": currency,
        "quantity": quantity,
    })
}

async fn create(db: &TestDb, b: Value) -> Answer {
    call(
        db,
        Some(CUSTOMER),
        Method::POST,
        "/api/bookings",
        Some(b),
        Some(&format!("k-r-bkc-{}", Uuid::new_v4())),
    )
    .await
}

async fn bookings_on(db: &TestDb, slot_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM bookings WHERE slot_id = $1")
        .bind(slot_id)
        .fetch_one(&db.pool)
        .await
        .expect("count bookings")
}

async fn reserved_on(db: &TestDb, slot_id: Uuid) -> i32 {
    sqlx::query_scalar("SELECT reserved FROM booking_slots WHERE id = $1")
        .bind(slot_id)
        .fetch_one(&db.pool)
        .await
        .expect("read slot reserved")
}

fn accepted(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{what}: expected 200; got {} {}",
        a.status,
        a.text
    );
}

/// 400 validationFailed on `field`, and the slot holds no booking and no reservation.
async fn refused_on(db: &TestDb, slot_id: Uuid, a: &Answer, field: &str, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST
            && a.body["error"]["code"] == "validationFailed"
            && a.body["error"]["field"] == field,
        "{what}: expected 400 validationFailed on `{field}`; got {} {}",
        a.status,
        a.text
    );
    assert_eq!(bookings_on(db, slot_id).await, 0, "{what}: nothing written");
    assert_eq!(
        reserved_on(db, slot_id).await,
        0,
        "{what}: the slot keeps its seats"
    );
}

// ------------------------------------------------------------ quantity: magnitude, both sides

#[actix_web::test]
async fn the_largest_quantity_the_column_holds_is_accepted() {
    let db = TestDb::start().await;
    let slot = open_slot(&db).await;
    let a = create(&db, body(slot, "USD", json!("9999999999999999.9999"))).await;
    accepted(&a, "quantity 9999999999999999.9999");
    let stored: String =
        sqlx::query_scalar("SELECT requested_quantity::text FROM bookings WHERE slot_id = $1")
            .bind(slot)
            .fetch_one(&db.pool)
            .await
            .expect("read stored quantity");
    assert_eq!(stored, "9999999999999999.9999");
    db.finish().await;
}

#[actix_web::test]
async fn a_quantity_of_ten_to_the_sixteen_is_refused_on_quantity() {
    let db = TestDb::start().await;
    let slot = open_slot(&db).await;
    let a = create(&db, body(slot, "USD", json!("10000000000000000"))).await;
    refused_on(&db, slot, &a, "quantity", "quantity 10^16").await;
    db.finish().await;
}

// ------------------------------------------------------------ quantity: scale and sign, both sides

#[actix_web::test]
async fn four_decimals_are_accepted_and_a_fifth_non_zero_one_is_refused() {
    let db = TestDb::start().await;
    let ok_slot = open_slot(&db).await;
    accepted(
        &create(&db, body(ok_slot, "USD", json!("0.0001"))).await,
        "quantity 0.0001",
    );
    let slot = open_slot(&db).await;
    let a = create(&db, body(slot, "USD", json!("0.00001"))).await;
    refused_on(
        &db,
        slot,
        &a,
        "quantity",
        "quantity 0.00001 (rounds to 0 in the column)",
    )
    .await;
    db.finish().await;
}

#[actix_web::test]
async fn zero_and_negative_quantities_are_refused_on_quantity() {
    let db = TestDb::start().await;
    for q in ["0", "-1", "-0.0001"] {
        let slot = open_slot(&db).await;
        let a = create(&db, body(slot, "USD", json!(q))).await;
        refused_on(&db, slot, &a, "quantity", &format!("quantity {q}")).await;
    }
    db.finish().await;
}

#[actix_web::test]
async fn a_quantity_that_is_not_a_plain_decimal_is_refused_on_quantity() {
    let db = TestDb::start().await;
    for q in ["1e5", "abc", " 1", ""] {
        let slot = open_slot(&db).await;
        let a = create(&db, body(slot, "USD", json!(q))).await;
        refused_on(&db, slot, &a, "quantity", &format!("quantity {q:?}")).await;
    }
    db.finish().await;
}

// ------------------------------------------------------------ quantity as a JSON number (kept during the change)

#[actix_web::test]
async fn a_json_number_quantity_is_still_accepted() {
    let db = TestDb::start().await;
    let slot = open_slot(&db).await;
    accepted(
        &create(&db, body(slot, "USD", json!(100))).await,
        "quantity 100 (number)",
    );
    let slot = open_slot(&db).await;
    accepted(
        &create(&db, body(slot, "USD", json!(250.5))).await,
        "quantity 250.5 (number)",
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_json_number_quantity_out_of_range_is_refused_on_quantity() {
    let db = TestDb::start().await;
    let slot = open_slot(&db).await;
    let a = create(&db, body(slot, "USD", json!(10000000000000000_u64))).await;
    refused_on(&db, slot, &a, "quantity", "quantity 10^16 as a number").await;
    db.finish().await;
}

// ------------------------------------------------------------ currency must name a row (B-7)

#[actix_web::test]
async fn an_unknown_currency_is_refused_on_currency_before_the_write() {
    let db = TestDb::start().await;
    for c in ["XXX", "usd", "US"] {
        let slot = open_slot(&db).await;
        let a = create(&db, body(slot, c, json!("100"))).await;
        refused_on(&db, slot, &a, "currency", &format!("currency {c:?}")).await;
    }
    db.finish().await;
}

#[actix_web::test]
async fn a_known_currency_is_accepted() {
    let db = TestDb::start().await;
    let slot = open_slot(&db).await;
    accepted(
        &create(&db, body(slot, "JPY", json!("100"))).await,
        "currency JPY",
    );
    db.finish().await;
}
