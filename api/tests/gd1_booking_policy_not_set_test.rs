//! GD-1b up-front tests (design `docs/design/blueprint/owner-set-values.md` §3 row 19, §6 D-3 and D-4): an
//! organisation's booking policy has no code fallback. With no `booking_policies` row:
//! - desk and kiosk check-in are refused `400 bookingsClosed`, as booking create already is (`booking_service.rs`);
//! - the readers keep no grace (`GRACE_UNTIL_SQL` gives NULL): a CONFIRMED booking stays CONFIRMED in `my_bookings`,
//!   `desk_bookings` and the QR re-issue, which is what the sweeper does (it joins `booking_policies`, `sweeper.rs`
//!   step 2, so it never marks such a booking NO_SHOW).
//!
//! With a row, every reader uses its `no_show_grace_minutes` (D-4: 20 here, not 15).
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add
//! tests in a file of your own. The test organisation's row is the migration's backfill (60 / 15 / 60), so each case
//! deletes or updates it first.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use bk6_common::*;

async fn no_booking_policy(db: &TestDb) {
    sqlx::query("DELETE FROM booking_policies WHERE organization_id = $1::uuid")
        .bind(ORG)
        .execute(&db.pool)
        .await
        .expect("delete the organisation's booking policy");
}

async fn grace_of_20_minutes(db: &TestDb) {
    sqlx::query(
        "INSERT INTO booking_policies (organization_id, cancel_cutoff_minutes, no_show_grace_minutes, \
             confirm_expiry_minutes) VALUES ($1::uuid, 60, 20, 60) \
         ON CONFLICT (organization_id) DO UPDATE SET no_show_grace_minutes = 20",
    )
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("a 20-minute grace");
}

/// A CONFIRMED booking of Alice's at `B01` in a one-hour slot starting at `start`, whose QR is `id`.
async fn confirmed(db: &TestDb, id: &str, no: &str, start: DateTime<Utc>) -> Uuid {
    let slot_id = Uuid::new_v4();
    seed_slot(
        &db.pool,
        slot_id,
        B01,
        bangkok_today(),
        start,
        start + Duration::hours(1),
        5,
        1,
        "OPEN",
    )
    .await;
    seed_booking(
        &db.pool,
        id,
        no,
        CUSTOMER_ALICE_CUST_ID,
        B01,
        slot_id,
        "APP",
        "CONFIRMED",
        &Sha256::digest(id.as_bytes()),
    )
    .await;
    slot_id
}

async fn desk_check_in(db: &TestDb, id: &str) -> Answer {
    call(
        db,
        Some("TELLER@XC-B01"),
        Method::POST,
        &format!("/api/bookings/{id}/check-in"),
        Some(json!({ "identityChecked": true })),
        Some(&unique("k-gd1-desk")),
    )
    .await
}

async fn kiosk_check_in(db: &TestDb, id: &str) -> Answer {
    call(
        db,
        Some("TELLER@XC-B01"),
        Method::POST,
        "/api/kiosk/bookings/check-in",
        Some(json!({ "branch": B01, "qrToken": id })),
        Some(&unique("k-gd1-kiosk")),
    )
    .await
}

async fn stored_status(db: &TestDb, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM bookings WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("the stored booking")
}

async fn booking_tickets(db: &TestDb, id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM queue_tickets WHERE booking_id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("count the booking's tickets")
}

/// 400 `bookingsClosed`; the booking stays CONFIRMED with no ticket.
async fn closed(db: &TestDb, res: &Answer, id: &str, what: &str) {
    assert_eq!(
        (res.status, res.body["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("bookingsClosed")),
        "{what}: expected 400 bookingsClosed; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(stored_status(db, id).await, "CONFIRMED", "{what}");
    assert_eq!(booking_tickets(db, id).await, 0, "{what}: no ticket");
}

/// 200 and CHECKED_IN.
async fn checked_in(db: &TestDb, res: &Answer, id: &str, what: &str) {
    assert_eq!(
        res.status,
        StatusCode::OK,
        "{what}: expected 200; got {} {}",
        res.status,
        res.text
    );
    assert_eq!(stored_status(db, id).await, "CHECKED_IN", "{what}");
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

async fn my_bookings_row(db: &TestDb, id: &str) -> Value {
    let res = call(
        db,
        Some("CUSTOMER@customer-alice"),
        Method::GET,
        &format!("/api/v3/views/my_bookings?branch={B01}"),
        None,
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "my_bookings: {}", res.text);
    item(&res.body, id)
        .unwrap_or_else(|| panic!("{id} is listed: {}", res.text))
        .clone()
}

async fn desk_bookings_row(db: &TestDb, id: &str) -> Value {
    let res = call(
        db,
        Some("TELLER@XC-B01"),
        Method::GET,
        &format!("/api/v3/views/desk_bookings?branch={B01}"),
        None,
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "desk_bookings: {}", res.text);
    item(&res.body, id)
        .unwrap_or_else(|| panic!("{id} is listed: {}", res.text))
        .clone()
}

// ------------------------------------------------------------ D-3: no booking_policies row

#[actix_web::test]
async fn with_no_booking_policy_desk_check_in_is_bookings_closed() {
    let db = TestDb::start().await;
    no_booking_policy(&db).await;
    let id = "BK-GD1-desk-closed";
    confirmed(&db, id, "B261103-0101", Utc::now() + Duration::minutes(5)).await;

    let res = desk_check_in(&db, id).await;
    closed(&db, &res, id, "desk check-in with no booking policy").await;
    db.finish().await;
}

#[actix_web::test]
async fn with_no_booking_policy_kiosk_check_in_is_bookings_closed() {
    let db = TestDb::start().await;
    no_booking_policy(&db).await;
    let id = "BK-GD1-kiosk-closed";
    confirmed(&db, id, "B261103-0102", Utc::now() + Duration::minutes(5)).await;

    let res = kiosk_check_in(&db, id).await;
    closed(&db, &res, id, "kiosk check-in with no booking policy").await;
    db.finish().await;
}

#[actix_web::test]
async fn with_no_booking_policy_my_bookings_keeps_a_past_confirmed_booking_confirmed() {
    let db = TestDb::start().await;
    no_booking_policy(&db).await;
    let id = "BK-GD1-my-past";
    confirmed(&db, id, "B261103-0103", Utc::now() - Duration::hours(2)).await;

    let row = my_bookings_row(&db, id).await;
    assert_eq!(
        row["status"], "CONFIRMED",
        "no grace: never NO_SHOW, as the sweeper: {row}"
    );
    db.finish().await;
}

#[actix_web::test]
async fn with_no_booking_policy_desk_bookings_keeps_a_past_confirmed_booking_confirmed() {
    let db = TestDb::start().await;
    no_booking_policy(&db).await;
    let id = "BK-GD1-desk-past";
    confirmed(&db, id, "B261103-0104", Utc::now() - Duration::hours(2)).await;

    let row = desk_bookings_row(&db, id).await;
    assert_eq!(row["status"], "CONFIRMED", "{row}");
    db.finish().await;
}

#[actix_web::test]
async fn with_no_booking_policy_a_past_confirmed_booking_is_not_overdue_for_the_qr_reissue() {
    let db = TestDb::start().await;
    no_booking_policy(&db).await;
    // The re-issue derives the new QR from the booking's uuid, so the id is `BK-<uuid>`.
    let id = &format!("BK-{}", Uuid::new_v4());
    confirmed(&db, id, "B261103-0105", Utc::now() - Duration::hours(2)).await;

    let res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &format!("/api/bookings/{id}/qr"),
        Some(json!({ "branch": B01 })),
        Some(&unique("k-gd1-qr")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "no grace ⇒ still CONFIRMED, so not 409 staleVersion; got {} {}",
        res.status,
        res.text
    );
    db.finish().await;
}

// ------------------------------------------------------------ D-4: the row's grace, everywhere

#[actix_web::test]
async fn a_20_minute_grace_admits_desk_check_in_18_minutes_after_the_start() {
    let db = TestDb::start().await;
    grace_of_20_minutes(&db).await;
    let id = "BK-GD1-desk-20";
    confirmed(&db, id, "B261103-0201", Utc::now() - Duration::minutes(18)).await;

    let res = desk_check_in(&db, id).await;
    checked_in(&db, &res, id, "desk check-in inside a 20-minute grace").await;
    db.finish().await;
}

#[actix_web::test]
async fn a_20_minute_grace_admits_kiosk_check_in_18_minutes_after_the_start() {
    let db = TestDb::start().await;
    grace_of_20_minutes(&db).await;
    let id = "BK-GD1-kiosk-20";
    confirmed(&db, id, "B261103-0202", Utc::now() - Duration::minutes(18)).await;

    let res = kiosk_check_in(&db, id).await;
    checked_in(&db, &res, id, "kiosk check-in inside a 20-minute grace").await;
    db.finish().await;
}

#[actix_web::test]
async fn a_20_minute_grace_decides_overdue_in_both_booking_views() {
    let db = TestDb::start().await;
    grace_of_20_minutes(&db).await;
    let inside = "BK-GD1-view-18";
    let outside = "BK-GD1-view-25";
    confirmed(
        &db,
        inside,
        "B261103-0203",
        Utc::now() - Duration::minutes(18),
    )
    .await;
    confirmed(
        &db,
        outside,
        "B261103-0204",
        Utc::now() - Duration::minutes(25),
    )
    .await;

    assert_eq!(my_bookings_row(&db, inside).await["status"], "CONFIRMED");
    assert_eq!(my_bookings_row(&db, outside).await["status"], "NO_SHOW");
    assert_eq!(desk_bookings_row(&db, inside).await["status"], "CONFIRMED");
    assert_eq!(desk_bookings_row(&db, outside).await["status"], "NO_SHOW");
    db.finish().await;
}

#[actix_web::test]
async fn a_20_minute_grace_decides_overdue_for_the_qr_reissue() {
    let db = TestDb::start().await;
    grace_of_20_minutes(&db).await;
    let inside = format!("BK-{}", Uuid::new_v4());
    let outside = format!("BK-{}", Uuid::new_v4());
    confirmed(
        &db,
        &inside,
        "B261103-0205",
        Utc::now() - Duration::minutes(18),
    )
    .await;
    confirmed(
        &db,
        &outside,
        "B261103-0206",
        Utc::now() - Duration::minutes(25),
    )
    .await;

    let ok_res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &format!("/api/bookings/{inside}/qr"),
        Some(json!({ "branch": B01 })),
        Some(&unique("k-gd1-qr-20-in")),
    )
    .await;
    assert_eq!(
        ok_res.status,
        StatusCode::OK,
        "18m inside 20m grace ⇒ 200; got {} {}",
        ok_res.status,
        ok_res.text
    );

    let err_res = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        &format!("/api/bookings/{outside}/qr"),
        Some(json!({ "branch": B01 })),
        Some(&unique("k-gd1-qr-20-out")),
    )
    .await;
    assert_eq!(
        (err_res.status, err_res.body["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("staleVersion")),
        "25m past start with 20m grace ⇒ 409 staleVersion; got {} {}",
        err_res.status,
        err_res.text
    );
    db.finish().await;
}
