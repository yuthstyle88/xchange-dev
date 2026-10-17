//! BK-13 (design `docs/design/blueprint/booking-slots.md` B12, §5.2): the booking views show an overdue row as its
//! final state, so a late sweep never shows a wrong state, and the customer's view never hands out a QR for it.
//! Written before the implementation by the ticket author (claude/c4d72c). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! The stored row is not changed by a read: writing stays the sweeper's (B12). The organisation's grace is set to
//! 15 minutes here; every overdue case is far past its boundary, the boundaries themselves are
//! `crates/xc-booking/tests/effective_status_test.rs`'s.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

/// The organisation's booking policy, with a 15-minute no-show grace.
async fn grace_of_15_minutes(db: &TestDb) {
    sqlx::query(
        "INSERT INTO booking_policies (organization_id, cancel_cutoff_minutes, no_show_grace_minutes, \
             confirm_expiry_minutes) VALUES ($1::uuid, 60, 15, 60) \
         ON CONFLICT (organization_id) DO UPDATE SET no_show_grace_minutes = 15",
    )
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("the booking policy");
}

/// A slot of `B01` on `date`, from `start` for an hour.
async fn slot(db: &TestDb, date: chrono::NaiveDate, start: chrono::DateTime<Utc>) -> Uuid {
    let id = Uuid::new_v4();
    seed_slot(
        &db.pool,
        id,
        B01,
        date,
        start,
        start + Duration::hours(1),
        5,
        1,
        "OPEN",
    )
    .await;
    id
}

async fn booking(db: &TestDb, id: &str, no: &str, slot_id: Uuid, status: &str) {
    let qr_hash = Sha256::digest(id.as_bytes());
    seed_booking(
        &db.pool,
        id,
        no,
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        status,
        &qr_hash,
    )
    .await;
}

/// The object whose `id` is `id`, wherever it is in the answer.
fn item<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
    match v {
        Value::Object(map) => {
            if map.get("id").and_then(Value::as_str) == Some(id) {
                return Some(v);
            }
            map.values().find_map(|x| item(x, id))
        }
        Value::Array(list) => list.iter().find_map(|x| item(x, id)),
        _ => None,
    }
}

async fn my_bookings(db: &TestDb) -> Value {
    let uri = format!("/api/v3/views/my_bookings?branch={B01}");
    let res = call(
        db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &uri,
        None,
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "my_bookings: {}", res.text);
    res.body
}

async fn stored_status(db: &TestDb, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM bookings WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("the stored booking")
}

fn no_qr(row: &Value, what: &str) {
    assert!(
        row.get("qrToken").is_none_or(Value::is_null),
        "{what}: no QR for an overdue booking: {row}"
    );
}

#[actix_web::test]
async fn a_confirmed_booking_past_start_plus_grace_shows_as_no_show_with_no_qr() {
    let db = TestDb::start().await;
    grace_of_15_minutes(&db).await;
    let now = Utc::now();
    let overdue = slot(&db, bangkok_today(), now - Duration::hours(2)).await;
    booking(
        &db,
        "BK-13-overdue-confirmed",
        "B261016-1301",
        overdue,
        "CONFIRMED",
    )
    .await;

    let body = my_bookings(&db).await;
    let row = item(&body, "BK-13-overdue-confirmed").expect("the booking is listed");
    assert_eq!(row["status"], "NO_SHOW", "{row}");
    no_qr(row, "CONFIRMED two hours after its start");
    assert_eq!(
        stored_status(&db, "BK-13-overdue-confirmed").await,
        "CONFIRMED",
        "a read writes nothing: the sweeper writes"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_confirmed_booking_before_its_start_keeps_its_status_and_its_qr() {
    let db = TestDb::start().await;
    grace_of_15_minutes(&db).await;
    let now = Utc::now();
    let upcoming = slot(&db, bangkok_today(), now + Duration::hours(1)).await;
    booking(&db, "BK-13-upcoming", "B261016-1302", upcoming, "CONFIRMED").await;

    let body = my_bookings(&db).await;
    let row = item(&body, "BK-13-upcoming").expect("the booking is listed");
    assert_eq!(row["status"], "CONFIRMED", "{row}");
    assert!(
        row["qrToken"].is_string(),
        "a live booking keeps its QR: {row}"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_pending_booking_past_its_confirm_expiry_shows_as_expired_with_no_qr() {
    let db = TestDb::start().await;
    grace_of_15_minutes(&db).await;
    let now = Utc::now();
    let later = slot(&db, bangkok_today(), now + Duration::hours(3)).await;
    booking(
        &db,
        "BK-13-pending",
        "B261016-1303",
        later,
        "PENDING_CUSTOMER_CONFIRMATION",
    )
    .await;
    sqlx::query("UPDATE bookings SET confirm_expires_at = $2 WHERE id = $1")
        .bind("BK-13-pending")
        .bind(now - Duration::minutes(30))
        .execute(&db.pool)
        .await
        .expect("an expired confirmation window");

    let body = my_bookings(&db).await;
    let row = item(&body, "BK-13-pending").expect("the booking is listed");
    assert_eq!(row["status"], "EXPIRED", "{row}");
    no_qr(row, "PENDING half an hour past its confirm expiry");
    assert_eq!(
        stored_status(&db, "BK-13-pending").await,
        "PENDING_CUSTOMER_CONFIRMATION"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_checked_in_booking_of_an_earlier_day_with_no_ticket_shows_as_cancelled_with_no_qr() {
    let db = TestDb::start().await;
    grace_of_15_minutes(&db).await;
    let yesterday = bangkok_today() - Duration::days(1);
    let start = Utc::now() - Duration::hours(26);
    let earlier = slot(&db, yesterday, start).await;
    booking(
        &db,
        "BK-13-day-ended",
        "B261016-1304",
        earlier,
        "CHECKED_IN",
    )
    .await;

    let body = my_bookings(&db).await;
    let row = item(&body, "BK-13-day-ended").expect("the booking is listed");
    assert_eq!(row["status"], "CANCELLED", "{row}");
    no_qr(row, "CHECKED_IN on yesterday's slot with no active ticket");
    assert_eq!(stored_status(&db, "BK-13-day-ended").await, "CHECKED_IN");
    db.finish().await;
}

#[actix_web::test]
async fn the_desk_list_shows_an_overdue_confirmed_booking_as_no_show() {
    let db = TestDb::start().await;
    grace_of_15_minutes(&db).await;
    let now = Utc::now();
    let overdue = slot(&db, bangkok_today(), now - Duration::hours(2)).await;
    booking(
        &db,
        "BK-13-desk-overdue",
        "B261016-1305",
        overdue,
        "CONFIRMED",
    )
    .await;
    let upcoming = slot(&db, bangkok_today(), now + Duration::hours(1)).await;
    booking(
        &db,
        "BK-13-desk-upcoming",
        "B261016-1306",
        upcoming,
        "CONFIRMED",
    )
    .await;

    let uri = format!("/api/v3/views/desk_bookings?branch={B01}");
    let res = call(&db, Some("TELLER@XC-B01"), Method::GET, &uri, None, None).await;
    assert_eq!(res.status, StatusCode::OK, "desk_bookings: {}", res.text);
    let overdue_row = item(&res.body, "BK-13-desk-overdue").expect("the overdue booking is listed");
    assert_eq!(overdue_row["status"], "NO_SHOW", "{overdue_row}");
    let upcoming_row =
        item(&res.body, "BK-13-desk-upcoming").expect("the upcoming booking is listed");
    assert_eq!(upcoming_row["status"], "CONFIRMED", "{upcoming_row}");
    assert_eq!(stored_status(&db, "BK-13-desk-overdue").await, "CONFIRMED");
    db.finish().await;
}
