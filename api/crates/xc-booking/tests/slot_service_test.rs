//! BK-2 #1379 — `SlotService` (design `docs/design/blueprint/booking-slots.md` B2, §4.1 bullets, §4.4; §9.1 row
//! BK-2: *"re-running adds nothing; a rule change closes empty future slots and leaves slots with bookings; capacity
//! below `reserved` refused"*; ticket §7 refusal names, §8 `generate`, §9 edge cases).
//! Written before the implementation by the test writer (claude/dell-claude-4, on the BE lane controller
//! claude/ad05bf8d's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The dates are in 2030, so every slot is in the future. Needs `TEST_DATABASE_URL` (see `slot_common`); nothing
//! here skips.

mod slot_common;

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc, Weekday};
use slot_common::{SlotRow, TestDb, status_text};
use xc_booking::application::slot_service::Generated;
use xc_booking::domain::slot::branch_closure::BranchClosure;
use xc_booking::domain::slot::repository::SlotRepository;
use xc_booking::domain::slot::slot_error::SlotError;
use xc_booking::domain::slot::slot_generator::SlotGenerator;
use xc_booking::domain::slot::slot_rule::SlotRule;
use xc_kernel::ids::BranchId;
use xc_rate::domain::branch::branch_hours::BranchHours;

const BANGKOK: &str = "Asia/Bangkok";

fn monday() -> NaiveDate {
    let d = NaiveDate::from_ymd_opt(2030, 1, 7).expect("a date");
    assert_eq!(d.weekday(), Weekday::Mon);
    d
}

fn day(n: u64) -> NaiveDate {
    monday() + chrono::Days::new(n)
}

fn every_day(open: &str, close: &str) -> String {
    let d = format!(r#"[{{"open":"{open}","close":"{close}"}}]"#);
    format!(r#"{{"mon":{d},"tue":{d},"wed":{d},"thu":{d},"fri":{d},"sat":{d},"sun":{d}}}"#)
}

fn rule(horizon_days: i32) -> SlotRule {
    SlotRule {
        slot_minutes: 30,
        capacity: 3,
        horizon_days,
    }
}

fn bangkok(d: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
    let tz: chrono_tz::Tz = BANGKOK.parse().expect("a timezone");
    tz.from_local_datetime(&d.and_hms_opt(hour, minute, 0).expect("a time"))
        .single()
        .expect("one instant")
        .with_timezone(&Utc)
}

/// A Bangkok branch open 09:00–11:00 every day (four 30-minute slots a day) with `rule(horizon)` set.
async fn branch_with_rule(db: &TestDb, horizon: i32) -> BranchId {
    let b = db
        .add_branch(BANGKOK, Some(&every_day("09:00", "11:00")))
        .await;
    let admin = db.add_user().await;
    db.service()
        .set_rule(&b, &rule(horizon), admin)
        .await
        .expect("the rule is saved");
    b
}

fn find(rows: &[SlotRow], start: DateTime<Utc>) -> &SlotRow {
    rows.iter()
        .find(|r| r.2 == start)
        .unwrap_or_else(|| panic!("a slot starting {start}: {rows:?}"))
}

/// Take one place in the slot starting at `start`, committed.
async fn book(db: &TestDb, b: &BranchId, start: DateTime<Utc>) {
    let id = find(&db.slots(b).await, start).0;
    let repo = db.repo();
    let mut tx = db.pool.begin().await.expect("begin");
    repo.reserve_in(&mut tx, id).await.expect("a free place");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn generate_stores_the_planned_slots_with_the_rule_capacity() {
    let db = TestDb::start().await;
    let b = branch_with_rule(&db, 14).await;
    let got = db
        .service()
        .generate(&b, monday(), 3)
        .await
        .expect("generate");
    assert_eq!(
        got,
        Generated {
            added: 12,
            closed: 0
        },
        "four slots a day for three days"
    );
    let rows = db.slots(&b).await;
    let hours = BranchHours::new(
        BANGKOK,
        Some(&serde_json::from_str(&every_day("09:00", "11:00")).expect("json")),
    )
    .expect("hours");
    let planned = SlotGenerator::slots_for(&rule(14), &hours, &[], monday(), 3);
    assert_eq!(rows.len(), planned.len());
    for (row, p) in rows.iter().zip(&planned) {
        assert_eq!(
            (row.1, row.2, row.3),
            (p.service_date, p.start_at, p.end_at)
        );
        assert_eq!((row.4, row.5, row.6.as_str()), (3, 0, "OPEN"), "{row:?}");
    }
    assert_eq!(rows[0].2, bangkok(monday(), 9, 0));
    db.finish().await;
}

#[tokio::test]
async fn running_generate_again_adds_nothing() {
    let db = TestDb::start().await;
    let b = branch_with_rule(&db, 14).await;
    let service = db.service();
    service.generate(&b, monday(), 3).await.expect("generate");
    let before = db.slots(&b).await;
    let again = service
        .generate(&b, monday(), 3)
        .await
        .expect("generate again");
    assert_eq!(
        again,
        Generated {
            added: 0,
            closed: 0
        }
    );
    assert_eq!(db.slots(&b).await, before, "the same rows, ids and all");
    db.finish().await;
}

#[tokio::test]
async fn a_branch_with_no_rule_gets_no_slots() {
    // Q-B1 option A: a branch with no rule offers no slots (whether generate answers Ok or a refusal is open).
    let db = TestDb::start().await;
    let b = db
        .add_branch(BANGKOK, Some(&every_day("09:00", "11:00")))
        .await;
    let _ = db.service().generate(&b, monday(), 3).await;
    assert!(db.slots(&b).await.is_empty());
    db.finish().await;
}

#[tokio::test]
async fn a_closure_added_before_generating_leaves_that_date_without_slots() {
    let db = TestDb::start().await;
    let b = branch_with_rule(&db, 14).await;
    let admin = db.add_user().await;
    let service = db.service();
    service
        .add_closure(
            &b,
            &BranchClosure {
                closed_on: day(1),
                reason: "Holiday".to_owned(),
            },
            admin,
        )
        .await
        .expect("the closure is saved");
    let got = service.generate(&b, monday(), 3).await.expect("generate");
    assert_eq!(got.added, 8);
    let rows = db.slots(&b).await;
    assert!(rows.iter().all(|r| r.1 != day(1)), "{rows:?}");
    db.finish().await;
}

#[tokio::test]
async fn a_closure_added_after_generating_closes_that_dates_empty_slots_and_leaves_booked_ones() {
    let db = TestDb::start().await;
    let b = branch_with_rule(&db, 14).await;
    let admin = db.add_user().await;
    let service = db.service();
    service.generate(&b, monday(), 3).await.expect("generate");
    let booked = bangkok(day(1), 9, 30);
    book(&db, &b, booked).await;
    service
        .add_closure(
            &b,
            &BranchClosure {
                closed_on: day(1),
                reason: "Holiday".to_owned(),
            },
            admin,
        )
        .await
        .expect("the closure is saved");
    let got = service.generate(&b, monday(), 3).await.expect("generate");
    assert_eq!(
        got,
        Generated {
            added: 0,
            closed: 3
        }
    );
    let rows = db.slots(&b).await;
    assert_eq!(rows.len(), 12, "nothing deleted");
    for r in &rows {
        let expected = if r.1 == day(1) && r.2 != booked {
            "CLOSED"
        } else {
            "OPEN"
        };
        assert_eq!(r.6, expected, "{r:?}");
    }
    assert_eq!(find(&rows, booked).5, 1, "the booked place is kept");
    db.finish().await;
}

#[tokio::test]
async fn a_shorter_horizon_closes_empty_slots_past_it_and_leaves_booked_ones() {
    let db = TestDb::start().await;
    let b = branch_with_rule(&db, 14).await;
    let admin = db.add_user().await;
    let service = db.service();
    let first = service.generate(&b, monday(), 14).await.expect("generate");
    assert_eq!(first.added, 56);
    let booked = bangkok(day(5), 10, 0);
    book(&db, &b, booked).await;
    service
        .set_rule(&b, &rule(2), admin)
        .await
        .expect("the rule changes");
    let got = service.generate(&b, monday(), 14).await.expect("generate");
    assert_eq!(
        got,
        Generated {
            added: 0,
            closed: 12 * 4 - 1
        }
    );
    let rows = db.slots(&b).await;
    assert_eq!(rows.len(), 56, "nothing deleted");
    for r in &rows {
        let inside = r.1 < day(2);
        let expected = if inside || r.2 == booked {
            "OPEN"
        } else {
            "CLOSED"
        };
        assert_eq!(r.6, expected, "{r:?}");
    }
    assert_eq!(find(&rows, booked).5, 1);
    db.finish().await;
}

#[tokio::test]
async fn shorter_hours_close_the_empty_slots_that_no_longer_fit_and_leave_booked_ones() {
    let db = TestDb::start().await;
    let b = branch_with_rule(&db, 14).await;
    let service = db.service();
    service.generate(&b, monday(), 2).await.expect("generate");
    let booked = bangkok(monday(), 10, 30);
    book(&db, &b, booked).await;
    db.set_hours(&b, &every_day("09:00", "10:00")).await;
    let got = service.generate(&b, monday(), 2).await.expect("generate");
    assert_eq!(
        got,
        Generated {
            added: 0,
            closed: 3
        },
        "Monday 10:00, Tuesday 10:00 and 10:30 closed; Monday 10:30 is booked"
    );
    let rows = db.slots(&b).await;
    assert_eq!(rows.len(), 8, "nothing deleted");
    let late = |r: &&SlotRow| r.2 >= bangkok(r.1, 10, 0);
    for r in rows.iter().filter(late) {
        let expected = if r.2 == booked { "OPEN" } else { "CLOSED" };
        assert_eq!(r.6, expected, "{r:?}");
    }
    assert!(
        rows.iter().filter(|r| !late(r)).all(|r| r.6 == "OPEN"),
        "{rows:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn capacity_below_reserved_is_refused_and_changes_nothing() {
    let db = TestDb::start().await;
    let b = db.add_branch(BANGKOK, None).await;
    let start = bangkok(monday(), 9, 0);
    let slot = db.add_slot(&b, start, 30, 4, 2).await;
    let service = db.service();
    let err = service
        .set_capacity(slot, 1)
        .await
        .expect_err("below reserved");
    assert!(matches!(err, SlotError::CapacityBelowReserved), "{err:?}");
    assert_eq!(err.code(), "capacityBelowReserved");
    assert_eq!(db.slot_row(slot).await, (4, 2, "OPEN".to_owned()));
    let s = service
        .set_capacity(slot, 2)
        .await
        .expect("down to reserved");
    assert_eq!((s.capacity, s.reserved), (2, 2));
    let s = service.set_capacity(slot, 6).await.expect("up");
    assert_eq!((s.capacity, s.reserved), (6, 2));
    assert_eq!(db.slot_row(slot).await, (6, 2, "OPEN".to_owned()));
    db.finish().await;
}

#[tokio::test]
async fn a_slot_with_places_taken_cannot_be_closed_and_an_empty_one_can() {
    let db = TestDb::start().await;
    let b = db.add_branch(BANGKOK, None).await;
    let held = db.add_slot(&b, bangkok(monday(), 9, 0), 30, 2, 1).await;
    let empty = db.add_slot(&b, bangkok(monday(), 9, 30), 30, 2, 0).await;
    let service = db.service();
    let err = service.close(held).await.expect_err("it has bookings");
    assert!(matches!(err, SlotError::SlotHasBookings), "{err:?}");
    assert_eq!(err.code(), "slotHasBookings");
    assert_eq!(db.slot_row(held).await, (2, 1, "OPEN".to_owned()));
    let s = service.close(empty).await.expect("an empty slot closes");
    assert_eq!(status_text(&s), "CLOSED");
    assert_eq!(db.slot_row(empty).await, (2, 0, "CLOSED".to_owned()));
    db.finish().await;
}
