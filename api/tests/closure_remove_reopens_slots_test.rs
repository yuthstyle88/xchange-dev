//! #5392 (design `booking-slots.md` §4.6, BK-16): a closed slot records why (`closed_by`), and
//! `closureRemove` reopens only the slots that closure closed. Owner 2026-10-29:
//! *"อนุมัติ A เปิด slot กลับเฉพาะที่วันหยุดปิด"*. Every row goes through the routes.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use bk6_common::*;

/// (id, start_at, status, closed_by, reserved)
type Row = (Uuid, DateTime<Utc>, String, Option<String>, i32);

fn every_day(open: &str, close: &str) -> Value {
    let day = json!([{ "open": open, "close": close }]);
    json!({
        "mon": day, "tue": day, "wed": day, "thu": day, "fri": day, "sat": day, "sun": day,
    })
}

fn ok(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
}

async fn set_hours(db: &TestDb, hours: Value) {
    sqlx::query("UPDATE branches SET opening_hours = $1 WHERE id = $2")
        .bind(hours)
        .bind(B01)
        .execute(&db.pool)
        .await
        .expect("set opening_hours on B01");
}

async fn set_rule(db: &TestDb, slot_minutes: i32) {
    let a = call(
        db,
        Some("ADMIN@*"),
        Method::PUT,
        &format!("/api/branches/{B01}/booking-slot-rule"),
        Some(json!({
            "branch": B01,
            "slotMinutes": slot_minutes,
            "capacity": 2,
            "horizonDays": 7,
        })),
        Some(&unique("k-rule")),
    )
    .await;
    ok(&a, "slotRuleSet");
}

/// Hours 09:00–12:00 every day, 60-minute slots, 7 days ahead; D = today + 3 has slots at 09, 10 and 11.
async fn seed(db: &TestDb) -> NaiveDate {
    set_hours(db, every_day("09:00", "12:00")).await;
    set_rule(db, 60).await;
    let d = bangkok_today() + Duration::days(3);
    assert_eq!(open_count(db, d).await, 3, "D starts with 3 OPEN slots");
    d
}

async fn add_closure(db: &TestDb, d: NaiveDate) {
    let a = call(
        db,
        Some("ADMIN@*"),
        Method::POST,
        &format!("/api/branches/{B01}/closures"),
        Some(json!({ "branch": B01, "closedOn": d.to_string(), "reason": "Holiday" })),
        Some(&unique("k-clos-add")),
    )
    .await;
    ok(&a, "closureAdd");
}

async fn remove_closure(db: &TestDb, d: NaiveDate) {
    let a = call(
        db,
        Some("ADMIN@*"),
        Method::POST,
        &format!("/api/branches/{B01}/closures/{d}/remove"),
        Some(json!({ "branch": B01 })),
        Some(&unique("k-clos-rm")),
    )
    .await;
    ok(&a, "closureRemove");
}

async fn slot_command(db: &TestDb, slot: Uuid, verb: &str) {
    let a = call(
        db,
        Some("ADMIN@*"),
        Method::POST,
        &format!("/api/booking-slots/{slot}/{verb}"),
        Some(json!({ "branch": B01 })),
        Some(&unique("k-slot")),
    )
    .await;
    ok(&a, verb);
}

async fn rows_on(db: &TestDb, d: NaiveDate) -> Vec<Row> {
    sqlx::query_as(
        "SELECT id, start_at, status, closed_by, reserved FROM booking_slots \
         WHERE branch_id = $1 AND service_date = $2 ORDER BY start_at",
    )
    .bind(B01)
    .bind(d)
    .fetch_all(&db.pool)
    .await
    .expect("read slots of D (closed_by is M3's column)")
}

async fn row(db: &TestDb, id: Uuid) -> Row {
    sqlx::query_as(
        "SELECT id, start_at, status, closed_by, reserved FROM booking_slots WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("read one slot (closed_by is M3's column)")
}

async fn open_count(db: &TestDb, d: NaiveDate) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM booking_slots WHERE branch_id = $1 AND service_date = $2 AND status = 'OPEN'",
    )
    .bind(B01)
    .bind(d)
    .fetch_one(&db.pool)
    .await
    .expect("count OPEN slots of D")
}

fn state(r: &Row) -> (&str, Option<&str>) {
    (r.2.as_str(), r.3.as_deref())
}

// R1: add a closure on D, then remove it: every empty future slot of D is OPEN again with closed_by NULL,
// and D's open count is what it was before the add. The add itself writes closed_by = 'CLOSURE'.
#[actix_web::test]
async fn r1_removing_a_closure_reopens_the_slots_it_closed() {
    let db = TestDb::start().await;
    let d = seed(&db).await;
    let before = open_count(&db, d).await;

    add_closure(&db, d).await;
    for r in rows_on(&db, d).await {
        assert_eq!(
            state(&r),
            ("CLOSED", Some("CLOSURE")),
            "closureAdd closed {r:?}"
        );
    }

    remove_closure(&db, d).await;
    for r in rows_on(&db, d).await {
        assert_eq!(state(&r), ("OPEN", None), "closureRemove reopened {r:?}");
    }
    assert_eq!(
        open_count(&db, d).await,
        before,
        "D's open count is back to before"
    );

    db.finish().await;
}

// R2: an ADMIN's slotClose on S survives a closure added and removed on S's date; the others reopen.
#[actix_web::test]
async fn r2_a_hand_closed_slot_stays_closed_after_the_closure_is_removed() {
    let db = TestDb::start().await;
    let d = seed(&db).await;
    let s = rows_on(&db, d).await[0].0;

    slot_command(&db, s, "close").await;
    assert_eq!(
        state(&row(&db, s).await),
        ("CLOSED", Some("SLOT")),
        "slotClose writes SLOT"
    );

    add_closure(&db, d).await;
    remove_closure(&db, d).await;

    for r in rows_on(&db, d).await {
        if r.0 == s {
            assert_eq!(state(&r), ("CLOSED", Some("SLOT")), "S stays hand-closed");
        } else {
            assert_eq!(state(&r), ("OPEN", None), "other slot of D reopened: {r:?}");
        }
    }

    db.finish().await;
}

// R3: a slot of D with a booking stays OPEN throughout a closure added and removed.
#[actix_web::test]
async fn r3_a_booked_slot_stays_open_throughout() {
    let db = TestDb::start().await;
    let d = seed(&db).await;
    let t = rows_on(&db, d).await[0].0;

    let booked = call(
        &db,
        Some("CUSTOMER@customer-alice"),
        Method::POST,
        "/api/bookings",
        Some(json!({
            "branch": B01,
            "branchId": B01,
            "slotId": t,
            "shopSide": "BUY",
            "currency": "USD",
            "quantity": 100.0,
        })),
        Some(&unique("k-book")),
    )
    .await;
    ok(&booked, "bookingCreate on T");

    add_closure(&db, d).await;
    let mid = row(&db, t).await;
    assert_eq!(
        (state(&mid), mid.4),
        (("OPEN", None), 1),
        "T open while the closure stands"
    );

    remove_closure(&db, d).await;
    let after = row(&db, t).await;
    assert_eq!(
        (state(&after), after.4),
        (("OPEN", None), 1),
        "T open after the removal"
    );

    db.finish().await;
}

// R4: a rule change closes R (closed_by = 'RULE'); undoing the rule does not reopen it (out of scope),
// and a closure added and removed on R's date leaves it CLOSED although R is planned again.
#[actix_web::test]
async fn r4_a_rule_closed_slot_stays_closed_after_the_closure_is_removed() {
    let db = TestDb::start().await;
    let d = seed(&db).await;
    let ten = rows_on(&db, d).await[1].clone();

    // 90-minute slots plan 09:00 and 10:30: the 10:00 slot no longer fits and is closed by the rule.
    set_rule(&db, 90).await;
    assert_eq!(
        state(&row(&db, ten.0).await),
        ("CLOSED", Some("RULE")),
        "rule change writes RULE"
    );

    // Back to 60 minutes: 10:00 is planned again, still CLOSED by the rule.
    set_rule(&db, 60).await;
    assert_eq!(
        state(&row(&db, ten.0).await),
        ("CLOSED", Some("RULE")),
        "rule restore reopens nothing"
    );
    let nine = rows_on(&db, d).await[0].0;
    assert_eq!(
        state(&row(&db, nine).await),
        ("OPEN", None),
        "09:00 is open before the closure"
    );

    add_closure(&db, d).await;
    remove_closure(&db, d).await;

    assert_eq!(
        state(&row(&db, ten.0).await),
        ("CLOSED", Some("RULE")),
        "R stays closed by the rule"
    );
    assert_eq!(
        state(&row(&db, nine).await),
        ("OPEN", None),
        "the closure's 09:00 reopened"
    );

    db.finish().await;
}

// R5: a CLOSED row with closed_by NULL (closed before M3, reason unknown) is never reopened.
#[actix_web::test]
async fn r5_a_pre_migration_closed_slot_stays_closed() {
    let db = TestDb::start().await;
    let d = seed(&db).await;
    let old = rows_on(&db, d).await[2].0;
    sqlx::query("UPDATE booking_slots SET status = 'CLOSED' WHERE id = $1")
        .bind(old)
        .execute(&db.pool)
        .await
        .expect("close one slot the way rows before M3 were");

    add_closure(&db, d).await;
    remove_closure(&db, d).await;

    for r in rows_on(&db, d).await {
        if r.0 == old {
            assert_eq!(state(&r), ("CLOSED", None), "the pre-M3 row stays CLOSED");
        } else {
            assert_eq!(
                state(&r),
                ("OPEN", None),
                "the closure's slot reopened: {r:?}"
            );
        }
    }

    db.finish().await;
}

// R6: a slot of D that has already started is not reopened, even when the closure closed it.
#[actix_web::test]
async fn r6_a_started_slot_is_not_reopened() {
    let db = TestDb::start().await;
    set_hours(&db, every_day("00:00", "23:00")).await;
    set_rule(&db, 60).await;
    let today = bangkok_today();
    let rows = rows_on(&db, today).await;
    let started = rows[0].clone();
    assert!(
        started.1 <= Utc::now(),
        "today's 00:00 slot has started: {started:?}"
    );

    add_closure(&db, today).await;
    // The closure closed it while it was still ahead (closureAdd only closes slots that have not started).
    sqlx::query("UPDATE booking_slots SET status = 'CLOSED', closed_by = 'CLOSURE' WHERE id = $1")
        .bind(started.0)
        .execute(&db.pool)
        .await
        .expect("mark the started slot as closed by the closure");

    remove_closure(&db, today).await;

    assert_eq!(
        state(&row(&db, started.0).await),
        ("CLOSED", Some("CLOSURE")),
        "started slot stays closed"
    );
    for r in rows_on(&db, today).await {
        if r.1 > Utc::now() + Duration::minutes(1) {
            assert_eq!(
                state(&r),
                ("OPEN", None),
                "a slot still ahead reopened: {r:?}"
            );
        }
    }

    db.finish().await;
}

// R7: slotOpen on a slot the closure closed opens it and clears closed_by.
#[actix_web::test]
async fn r7_slot_open_on_a_closure_closed_slot_clears_closed_by() {
    let db = TestDb::start().await;
    let d = seed(&db).await;
    add_closure(&db, d).await;
    let s = rows_on(&db, d).await[0].0;
    assert_eq!(state(&row(&db, s).await), ("CLOSED", Some("CLOSURE")));

    slot_command(&db, s, "open").await;
    assert_eq!(
        state(&row(&db, s).await),
        ("OPEN", None),
        "slotOpen clears closed_by"
    );

    db.finish().await;
}
