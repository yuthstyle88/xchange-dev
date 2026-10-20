//! Ticket #3636 — `closureAdd` re-runs the slot generator over the branch's horizon in the same
//! transaction, and `slotRuleSet` propagates `generate_in` errors instead of dropping them.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, NaiveDate};
use serde_json::json;
use uuid::Uuid;

use bk6_common::*;

type SlotSnapshot = (Uuid, NaiveDate, String, i32);

async fn seed_branch_with_slots(db: &TestDb) -> NaiveDate {
    let hours = json!({
        "mon": [{"open": "09:00", "close": "12:00"}],
        "tue": [{"open": "09:00", "close": "12:00"}],
        "wed": [{"open": "09:00", "close": "12:00"}],
        "thu": [{"open": "09:00", "close": "12:00"}],
        "fri": [{"open": "09:00", "close": "12:00"}],
        "sat": [{"open": "09:00", "close": "12:00"}],
        "sun": [{"open": "09:00", "close": "12:00"}],
    });
    sqlx::query("UPDATE branches SET opening_hours = $1 WHERE id = $2")
        .bind(hours)
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("set opening_hours on B01");

    let res = call(
        db,
        Some("ADMIN@*"),
        Method::PUT,
        &format!("/api/branches/{B01}/booking-slot-rule"),
        Some(json!({
            "branch": B01,
            "slotMinutes": 60,
            "capacity": 2,
            "horizonDays": 7,
        })),
        Some(&unique("k-seed-rule")),
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "slotRuleSet seed must succeed 200; got {} {}",
        res.status,
        res.text
    );

    let day_d = bangkok_today() + Duration::days(3);
    let open_on_d: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM booking_slots \
         WHERE branch_id = $1 AND service_date = $2 AND status = 'OPEN' AND reserved = 0",
    )
    .bind(B01)
    .bind(day_d)
    .fetch_one(&db.pool)
    .await
    .expect("count open slots on D");
    assert_eq!(open_on_d, 3, "day D must start with 3 OPEN empty slots");

    day_d
}

async fn add_closure(db: &TestDb, closed_on: NaiveDate, reason: &str, key: &str) -> Answer {
    call(
        db,
        Some("ADMIN@*"),
        Method::POST,
        &format!("/api/branches/{B01}/closures"),
        Some(json!({
            "branch": B01,
            "closedOn": closed_on.to_string(),
            "reason": reason,
        })),
        Some(key),
    )
    .await
}

async fn slots_on(db: &TestDb, day: NaiveDate) -> Vec<SlotSnapshot> {
    sqlx::query_as(
        "SELECT id, service_date, status, reserved FROM booking_slots \
         WHERE branch_id = $1 AND service_date = $2 ORDER BY start_at",
    )
    .bind(B01)
    .bind(day)
    .fetch_all(&db.pool)
    .await
    .expect("read slots on day")
}

async fn slots_not_on(db: &TestDb, day: NaiveDate) -> Vec<SlotSnapshot> {
    sqlx::query_as(
        "SELECT id, service_date, status, reserved FROM booking_slots \
         WHERE branch_id = $1 AND service_date <> $2 ORDER BY start_at",
    )
    .bind(B01)
    .bind(day)
    .fetch_all(&db.pool)
    .await
    .expect("read slots on other days")
}

// Row 1: `closureAdd` for D -> 200; every slot on D with `reserved = 0` is `CLOSED`; slots on other days unchanged.
#[actix_web::test]
async fn row_1_closure_add_for_d_closes_empty_slots_on_d_and_leaves_other_days_unchanged() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let other_before = slots_not_on(&db, day_d).await;
    assert!(!other_before.is_empty(), "other days in horizon have slots");

    let res = add_closure(&db, day_d, "Holiday", "k-row1-clos").await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "closureAdd must succeed 200: {}",
        res.text
    );
    assert_eq!(res.body["ok"], json!(true));

    let on_d = slots_on(&db, day_d).await;
    assert_eq!(on_d.len(), 3);
    for (id, _, status, reserved) in &on_d {
        assert_eq!(*reserved, 0);
        assert_eq!(
            status, "CLOSED",
            "empty slot {id} on closed day D must be CLOSED"
        );
    }

    let other_after = slots_not_on(&db, day_d).await;
    assert_eq!(
        other_after, other_before,
        "slots on other days must remain unchanged"
    );

    db.finish().await;
}

// Row 2: a slot on D holds a booking (`reserved = 1`), then `closureAdd` for D ->
// that slot stays `OPEN` with its booking; the empty slots on D are `CLOSED`.
#[actix_web::test]
async fn row_2_slot_on_d_with_booking_stays_open_while_empty_slots_on_d_close() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let initial_d = slots_on(&db, day_d).await;
    let booked_slot_id = initial_d[0].0;

    let res_book = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(json!({
            "branch": B01,
            "branchId": B01,
            "slotId": booked_slot_id,
            "shopSide": "BUY",
            "currency": "USD",
            "quantity": 100.0,
        })),
        Some("k-row2-book"),
    )
    .await;
    assert_eq!(
        res_book.status,
        StatusCode::OK,
        "bookingCreate before closure must succeed: {}",
        res_book.text
    );

    let res_clos = add_closure(&db, day_d, "Holiday", "k-row2-clos").await;
    assert_eq!(
        res_clos.status,
        StatusCode::OK,
        "closureAdd must succeed 200: {}",
        res_clos.text
    );

    let after_d = slots_on(&db, day_d).await;
    assert_eq!(after_d.len(), 3);
    for (id, _, status, reserved) in after_d {
        if id == booked_slot_id {
            assert_eq!(reserved, 1, "booked slot keeps reserved = 1");
            assert_eq!(status, "OPEN", "booked slot stays OPEN");
        } else {
            assert_eq!(reserved, 0);
            assert_eq!(status, "CLOSED", "empty slot {id} on D is CLOSED");
        }
    }

    let booking_status: String =
        sqlx::query_scalar("SELECT status FROM bookings WHERE slot_id = $1")
            .bind(booked_slot_id)
            .fetch_one(&db.pool)
            .await
            .expect("read booking status");
    assert_eq!(booking_status, "CONFIRMED");

    db.finish().await;
}

// Row 3: `closureAdd` for D, then `bookingCreate` on a D slot -> refused `slotUnavailable`.
#[actix_web::test]
async fn row_3_closure_add_for_d_then_booking_create_on_d_slot_is_refused_slot_unavailable() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let slot_id = slots_on(&db, day_d).await[0].0;

    let res_clos = add_closure(&db, day_d, "Holiday", "k-row3-clos").await;
    assert_eq!(res_clos.status, StatusCode::OK, "{}", res_clos.text);

    let res_book = call(
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
        Some("k-row3-book"),
    )
    .await;
    // `booking_service.rs:200-202` returns `BookingError::SlotUnavailable` when `status != "OPEN"`,
    // which `commands/bookings/create.rs:213` maps to 400 `capacityFull` (or `slotUnavailable` via `From`).
    assert_eq!(
        res_book.status,
        StatusCode::BAD_REQUEST,
        "bookingCreate on a slot closed by closureAdd must be refused with 400; got {} {}",
        res_book.status,
        res_book.text
    );
    let err_code = res_book.body["error"]["code"].as_str().unwrap_or("");
    assert!(
        matches!(err_code, "slotUnavailable" | "capacityFull"),
        "expected slotUnavailable/capacityFull refusal; got {}",
        res_book.text
    );
    let bookings_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM bookings WHERE slot_id = $1")
            .bind(slot_id)
            .fetch_one(&db.pool)
            .await
            .expect("count bookings on closed slot");
    assert_eq!(bookings_count, 0, "no booking written on closed slot");

    db.finish().await;
}

// Row 4: `closureAdd` for a date beyond the horizon -> 200; no slot changes; the closure row exists.
#[actix_web::test]
async fn row_4_closure_add_beyond_horizon_leaves_slots_unchanged_and_inserts_closure() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let beyond = bangkok_today() + Duration::days(10);
    let before = slots_not_on(&db, beyond).await;
    assert!(slots_on(&db, day_d).await.iter().all(|s| s.2 == "OPEN"));

    let res = add_closure(&db, beyond, "Far Holiday", "k-row4-clos").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);

    let closure_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM branch_closures WHERE branch_id = $1 AND closed_on = $2",
    )
    .bind(B01)
    .bind(beyond)
    .fetch_one(&db.pool)
    .await
    .expect("count closure row");
    assert_eq!(
        closure_count, 1,
        "closure row for beyond-horizon date exists"
    );

    let after = slots_not_on(&db, beyond).await;
    assert_eq!(
        after, before,
        "no slot changes when closure is beyond horizon"
    );
    assert_eq!(slots_on(&db, beyond).await.len(), 0);

    db.finish().await;
}

// Row 5: `closureAdd` on a branch with no slot rule -> 200; the closure row exists; no slot rows.
#[actix_web::test]
async fn row_5_closure_add_on_branch_with_no_slot_rule_inserts_closure_and_no_slots() {
    let db = TestDb::start().await;
    sqlx::query("DELETE FROM booking_slot_rules WHERE branch_id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("remove seeded slot rule");

    let day_d = bangkok_today() + Duration::days(3);
    let res = add_closure(&db, day_d, "No-rule Holiday", "k-row5-clos").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.body["ok"], json!(true));

    let closure_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM branch_closures WHERE branch_id = $1 AND closed_on = $2",
    )
    .bind(B01)
    .bind(day_d)
    .fetch_one(&db.pool)
    .await
    .expect("count closure row");
    assert_eq!(closure_count, 1, "closure row exists");

    let slot_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM booking_slots WHERE branch_id = $1")
            .bind(B01)
            .fetch_one(&db.pool)
            .await
            .expect("count booking_slots");
    assert_eq!(slot_count, 0, "no slot rows generated without a slot rule");

    db.finish().await;
}

// Row 6: `slotRuleSet` when generation fails (e.g. the branch's `opening_hours` is not valid JSON
// hours, so `BranchHours::new` errors) -> not `200 {ok:true}`: the error is answered and the rule
// save rolls back.
#[actix_web::test]
async fn row_6_slot_rule_set_when_generation_fails_answers_error_and_rolls_back_rule() {
    let db = TestDb::start().await;
    sqlx::query("UPDATE branches SET opening_hours = '\"not-an-object\"'::jsonb WHERE id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("corrupt opening_hours on B01");

    let res = call(
        &db,
        Some("ADMIN@*"),
        Method::PUT,
        &format!("/api/branches/{B01}/booking-slot-rule"),
        Some(json!({
            "branch": B01,
            "slotMinutes": 60,
            "capacity": 2,
            "horizonDays": 7,
        })),
        Some("k-row6-rule"),
    )
    .await;
    assert_ne!(
        res.status,
        StatusCode::OK,
        "slotRuleSet must not return 200 when slot generation fails; got {} {}",
        res.status,
        res.text
    );
    assert_ne!(res.body["ok"], json!(true));

    let rule_row: (i32, i32, i32) = sqlx::query_as(
        "SELECT slot_minutes, capacity, horizon_days FROM booking_slot_rules WHERE branch_id = $1",
    )
    .bind(B01)
    .fetch_one(&db.pool)
    .await
    .expect("read booking_slot_rules");
    assert_eq!(
        rule_row,
        (30, 10, 14),
        "rule save must roll back to the previously seeded rule (30, 10, 14)"
    );

    db.finish().await;
}

// Row 7: `closureAdd` twice for D (same key, then a new key) -> 200 both; the slots on D stay
// `CLOSED`; one closure row.
#[actix_web::test]
async fn row_7_closure_add_twice_for_d_same_key_then_new_key_keeps_slots_closed_and_one_row() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;

    let first = add_closure(&db, day_d, "Holiday", "k-row7-first").await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);

    let replay = add_closure(&db, day_d, "Holiday", "k-row7-first").await;
    assert_eq!(replay.status, StatusCode::OK, "{}", replay.text);
    assert_eq!(replay.body["ok"], json!(true));

    let second = add_closure(&db, day_d, "Updated Holiday", "k-row7-second").await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text);
    assert_eq!(second.body["ok"], json!(true));

    let on_d = slots_on(&db, day_d).await;
    assert_eq!(on_d.len(), 3);
    assert!(
        on_d.iter().all(|(_, _, status, _)| status == "CLOSED"),
        "all slots on D stay CLOSED: {on_d:?}"
    );

    let closure_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM branch_closures WHERE branch_id = $1 AND closed_on = $2",
    )
    .bind(B01)
    .bind(day_d)
    .fetch_one(&db.pool)
    .await
    .expect("count closure rows");
    assert_eq!(closure_count, 1, "exactly one closure row for D");

    db.finish().await;
}
