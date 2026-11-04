//! `branchSave` re-runs the slot generator when it saves the branch's opening hours (#564 rehearsal: MAIN's hours set
//! to 12:00-18:00 through the UI, and the booking screen still offered 18:00-23:30).
//!
//! Written before the implementation by the ticket's test author (claude/b091f5e0, on the xchange lane controller
//! claude/022a7453's order). Do not edit, delete or weaken these tests; add your own in another file.
//!
//! [required] by these tests: `branchSave` calls `SlotService::generate_in` over the branch's horizon in the same
//! transaction, from the branch's today, as `slotRuleSet` and `closureAdd` do. So, after a save:
//! - hours narrowed: each empty future slot outside the new hours is `CLOSED` (`closed_by = 'RULE'`), and booking it is
//!   refused;
//! - a slot outside the new hours that holds a booking is untouched (`OPEN`, its places kept);
//! - hours widened: the new hours' slots are added `OPEN`;
//! - a branch with no `booking_slot_rules` row: the save still answers 200 and no slot is made;
//! - a refused save (a code another branch holds): no slot changes;
//! - hours widened again: a slot an hours change closed (`closed_by = 'RULE'`, empty, future, planned again) is `OPEN`
//!   again with `closed_by` NULL. A slot an admin closed (`'SLOT'`) or a closure closed (`'CLOSURE'`) stays `CLOSED`.
//!   Ruled by the xchange lane controller on #6079 (issuecomment-6624616081, 13:51 ICT 2026-11-04).
//!
//! The harness is `bk6_common`'s, as in `closure_add_closes_slots_test.rs`. Needs `TEST_DATABASE_URL`; nothing skips.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{Duration, NaiveDate};
use serde_json::{Value, json};
use uuid::Uuid;

use bk6_common::*;

type Slot = (Uuid, String, String, i32, Option<String>);

fn hours(open: &str, close: &str) -> Value {
    let day = json!([{ "open": open, "close": close }]);
    json!({ "mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": day })
}

fn save_body(code: &str, opening_hours: Value) -> Value {
    json!({
        "branch": B01,
        "code": code,
        "address": "1 Main Rd",
        "timezone": "Asia/Bangkok",
        "openingHours": opening_hours,
        "bookingEnabled": true,
    })
}

async fn branch_save(db: &TestDb, body: Value, key: &str) -> Answer {
    call(
        db,
        Some("ADMIN@*"),
        Method::PUT,
        "/api/branches/profile",
        Some(body),
        Some(&unique(key)),
    )
    .await
}

/// B01 open 09:00-12:00 every day, with a 60-minute rule of capacity 2 over 7 days, saved through `slotRuleSet` so
/// the slots are the generator's. Answers day D = today + 3, which starts with 09:00, 10:00 and 11:00 OPEN.
async fn seed_branch_with_slots(db: &TestDb) -> NaiveDate {
    sqlx::query("UPDATE branches SET opening_hours = $1, code = 'B01', timezone = 'Asia/Bangkok' WHERE id = $2")
        .bind(hours("09:00", "12:00"))
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("set opening_hours on B01");
    sqlx::query("ALTER TABLE booking_slot_rules DROP CONSTRAINT IF EXISTS booking_slot_rules_updated_by_fkey")
        .execute(&db.pool)
        .await
        .expect("drop fk");
    let res = call(
        db,
        Some("EXECUTIVE@*"),
        Method::PUT,
        &format!("/api/branches/{B01}/booking-slot-rule"),
        Some(json!({ "branch": B01, "slotMinutes": 60, "capacity": 2, "horizonDays": 7 })),
        Some(&unique("k-seed-rule")),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "slotRuleSet seed: {}", res.text);

    let day_d = bangkok_today() + Duration::days(3);
    assert_eq!(
        starts_on(db, day_d, "OPEN").await,
        ["09:00", "10:00", "11:00"],
        "day D starts with three OPEN slots"
    );
    day_d
}

/// Every slot of B01 on `day`: id, local start `HH:MI`, status, reserved, closed_by.
async fn slots_on(db: &TestDb, day: NaiveDate) -> Vec<Slot> {
    sqlx::query_as(
        "SELECT id, to_char(start_at AT TIME ZONE 'Asia/Bangkok', 'HH24:MI'), status, reserved, closed_by \
         FROM booking_slots WHERE branch_id = $1 AND service_date = $2 ORDER BY start_at",
    )
    .bind(B01)
    .bind(day)
    .fetch_all(&db.pool)
    .await
    .expect("read slots")
}

async fn starts_on(db: &TestDb, day: NaiveDate, status: &str) -> Vec<String> {
    slots_on(db, day)
        .await
        .into_iter()
        .filter(|s| s.2 == status)
        .map(|s| s.1)
        .collect()
}

async fn all_slots(db: &TestDb) -> Vec<Slot> {
    sqlx::query_as(
        "SELECT id, to_char(start_at AT TIME ZONE 'Asia/Bangkok', 'YYYY-MM-DD HH24:MI'), status, reserved, closed_by \
         FROM booking_slots WHERE branch_id = $1 ORDER BY start_at",
    )
    .bind(B01)
    .fetch_all(&db.pool)
    .await
    .expect("read all slots")
}

async fn book(db: &TestDb, slot: Uuid, key: &str) -> Answer {
    call(
        db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(json!({
            "branch": B01,
            "branchId": B01,
            "slotId": slot,
            "shopSide": "BUY",
            "currency": "USD",
            "quantity": 100.0,
        })),
        Some(&unique(key)),
    )
    .await
}

#[actix_web::test]
async fn hours_narrowed_close_the_empty_future_slots_outside_them_and_they_cannot_be_booked() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let eleven = slots_on(&db, day_d).await[2].0;

    let res = branch_save(&db, save_body("B01", hours("09:00", "11:00")), "k-narrow").await;
    assert_eq!(res.status, StatusCode::OK, "branchSave: {}", res.text);

    let after = slots_on(&db, day_d).await;
    assert_eq!(after.len(), 3, "a slot is never deleted: {after:?}");
    assert_eq!(
        starts_on(&db, day_d, "OPEN").await,
        ["09:00", "10:00"],
        "{after:?}"
    );
    assert_eq!(
        (
            after[2].1.as_str(),
            after[2].2.as_str(),
            after[2].4.as_deref()
        ),
        ("11:00", "CLOSED", Some("RULE")),
        "the 11:00 slot is outside 09:00-11:00: {after:?}"
    );

    let refused = book(&db, eleven, "k-narrow-book").await;
    assert!(
        refused.status.is_client_error(),
        "booking a slot outside the hours is refused: {} {}",
        refused.status,
        refused.text
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_slot_outside_the_new_hours_that_holds_a_booking_is_untouched() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let eleven = slots_on(&db, day_d).await[2].0;
    let booked = book(&db, eleven, "k-held-book").await;
    assert_eq!(
        booked.status,
        StatusCode::OK,
        "bookingCreate: {}",
        booked.text
    );

    let res = branch_save(&db, save_body("B01", hours("09:00", "11:00")), "k-held").await;
    assert_eq!(res.status, StatusCode::OK, "branchSave: {}", res.text);

    let after = slots_on(&db, day_d).await;
    let held = after
        .iter()
        .find(|s| s.0 == eleven)
        .expect("the booked slot");
    assert_eq!(
        (held.2.as_str(), held.3, held.4.as_deref()),
        ("OPEN", 1, None),
        "the booked slot keeps its status and its place: {after:?}"
    );
    db.finish().await;
}

#[actix_web::test]
async fn hours_widened_add_the_new_hours_slots_open() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;

    let res = branch_save(&db, save_body("B01", hours("09:00", "14:00")), "k-widen").await;
    assert_eq!(res.status, StatusCode::OK, "branchSave: {}", res.text);

    assert_eq!(
        starts_on(&db, day_d, "OPEN").await,
        ["09:00", "10:00", "11:00", "12:00", "13:00"],
        "{:?}",
        slots_on(&db, day_d).await
    );
    db.finish().await;
}

#[actix_web::test]
async fn hours_moved_to_another_part_of_the_day_close_the_old_and_add_the_new() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;

    let res = branch_save(&db, save_body("B01", hours("12:00", "15:00")), "k-move").await;
    assert_eq!(res.status, StatusCode::OK, "branchSave: {}", res.text);

    assert_eq!(
        starts_on(&db, day_d, "OPEN").await,
        ["12:00", "13:00", "14:00"]
    );
    assert_eq!(
        starts_on(&db, day_d, "CLOSED").await,
        ["09:00", "10:00", "11:00"]
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_branch_with_no_slot_rule_still_saves_and_gets_no_slots() {
    let db = TestDb::start().await;
    sqlx::query("UPDATE branches SET code = 'B01' WHERE id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("set code");
    // bk6_common seeds a rule for B01 (`TestDb::start`); this branch has none.
    sqlx::query("DELETE FROM booking_slot_rules WHERE branch_id = $1")
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("remove B01's slot rule");

    let res = branch_save(&db, save_body("B01", hours("09:00", "12:00")), "k-norule").await;
    assert_eq!(res.status, StatusCode::OK, "branchSave: {}", res.text);
    assert_eq!(
        res.body["openingHours"],
        hours("09:00", "12:00"),
        "{}",
        res.text
    );
    assert!(all_slots(&db).await.is_empty(), "no rule, no slots");
    db.finish().await;
}

#[actix_web::test]
async fn a_refused_save_changes_no_slot() {
    let db = TestDb::start().await;
    seed_branch_with_slots(&db).await;
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id, booking_enabled, code) \
         VALUES ('XC-B02', 'Branch 2', $1::uuid, true, 'TAKEN')",
    )
    .bind(ORG)
    .execute(&db.pool)
    .await
    .expect("insert a branch holding the code TAKEN");
    let before = all_slots(&db).await;

    let res = branch_save(&db, save_body("TAKEN", hours("09:00", "10:00")), "k-taken").await;
    assert_eq!(
        res.status,
        StatusCode::CONFLICT,
        "a code another branch holds: {}",
        res.text
    );
    assert_eq!(
        all_slots(&db).await,
        before,
        "a refused save leaves every slot as it was"
    );
    db.finish().await;
}

/// Sets one slot of B01 to `CLOSED` with `closed_by`, as `slotClose` (`SLOT`) or `closureAdd` (`CLOSURE`) would.
async fn close_as(db: &TestDb, slot: Uuid, closed_by: &str) {
    sqlx::query("UPDATE booking_slots SET status = 'CLOSED', closed_by = $2 WHERE id = $1")
        .bind(slot)
        .bind(closed_by)
        .execute(&db.pool)
        .await
        .expect("close the slot");
}

async fn state_of(db: &TestDb, slot: Uuid) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, closed_by FROM booking_slots WHERE id = $1")
        .bind(slot)
        .fetch_one(&db.pool)
        .await
        .expect("read the slot")
}

#[actix_web::test]
async fn hours_narrowed_then_widened_back_reopen_the_slots_the_narrowing_closed() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let before = slots_on(&db, day_d).await;

    let res = branch_save(
        &db,
        save_body("B01", hours("09:00", "10:00")),
        "k-back-narrow",
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "branchSave narrow: {}",
        res.text
    );
    assert_eq!(starts_on(&db, day_d, "CLOSED").await, ["10:00", "11:00"]);

    let res = branch_save(
        &db,
        save_body("B01", hours("09:00", "12:00")),
        "k-back-widen",
    )
    .await;
    assert_eq!(
        res.status,
        StatusCode::OK,
        "branchSave widen back: {}",
        res.text
    );

    let after = slots_on(&db, day_d).await;
    assert_eq!(
        after
            .iter()
            .map(|s| (s.0, s.2.as_str(), s.4.as_deref()))
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|s| (s.0, "OPEN", None))
            .collect::<Vec<_>>(),
        "the same three rows, OPEN again with no closed_by: {after:?}"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_slot_an_admin_closed_stays_closed_when_the_hours_widen() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let ten = slots_on(&db, day_d).await[1].0;
    close_as(&db, ten, "SLOT").await;

    let res = branch_save(
        &db,
        save_body("B01", hours("09:00", "14:00")),
        "k-slot-widen",
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "branchSave widen: {}", res.text);

    assert_eq!(
        state_of(&db, ten).await,
        ("CLOSED".to_owned(), Some("SLOT".to_owned())),
        "an admin's close is never undone by an hours change"
    );
    assert_eq!(
        starts_on(&db, day_d, "OPEN").await,
        ["09:00", "11:00", "12:00", "13:00"],
        "the widen still adds the new hours"
    );
    db.finish().await;
}

#[actix_web::test]
async fn a_slot_a_closure_closed_stays_closed_when_the_hours_widen() {
    let db = TestDb::start().await;
    let day_d = seed_branch_with_slots(&db).await;
    let ten = slots_on(&db, day_d).await[1].0;
    close_as(&db, ten, "CLOSURE").await;

    let res = branch_save(
        &db,
        save_body("B01", hours("09:00", "14:00")),
        "k-closure-widen",
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "branchSave widen: {}", res.text);

    assert_eq!(
        state_of(&db, ten).await,
        ("CLOSED".to_owned(), Some("CLOSURE".to_owned())),
        "only closureRemove reopens what a closure closed (booking-slots.md §4.6)"
    );
    db.finish().await;
}
