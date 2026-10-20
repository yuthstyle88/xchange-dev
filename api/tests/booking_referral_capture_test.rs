// Written before the implementation by the ticket author claude/5edf7bb5 for this ticket. Do not edit, delete or weaken.

//! booking-slots.md B13: a referralCode given on an APP booking is captured on the booking in the same transaction.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

use bk6_common::*;

#[actix_web::test]
async fn app_booking_with_referral_code_stores_the_attribution() {
    let db = TestDb::start().await;
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

    let payload = json!({
        "branch": B01, "branchId": B01, "slotId": slot_id,
        "shopSide": "BUY", "currency": "USD", "quantity": 100.0,
        "referralCode": "test1234",
    });
    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(payload),
        Some("k-ref-1"),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "bookingCreate with a referral code: {} {}",
        res.status,
        res.text
    );
    let booking_id = res.body["id"]
        .as_str()
        .or_else(|| res.body["data"]["id"].as_str())
        .expect("the answer carries the booking id")
        .to_owned();

    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT code FROM referral_attributions WHERE booking_id::text = $1 AND status = 'ACTIVE'",
    )
    .bind(&booking_id)
    .fetch_all(&db.pool)
    .await
    .expect("read referral_attributions");
    assert_eq!(
        rows,
        vec![("TEST1234".to_string(),)],
        "one ACTIVE attribution, code trimmed and upper-cased"
    );

    db.finish().await;
}
