//! BK-2 #1379 — Ruling 2: when `slot_minutes` changes, an empty slot whose `start_at` equals a new slot's
//! start is reshaped in place: its `end_at` and capacity are updated to the new rule.
//! A reshape never touches status (a teller's close survives generate).
//! Reshaping applies only to future slots (start_at > now).
//! A slot with bookings keeps its old shape. Nothing is deleted.

mod slot_common;

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc, Weekday};
use slot_common::{SlotRow, TestDb};
use xc_booking::application::slot_service::Generated;
use xc_booking::domain::slot::repository::SlotRepository;
use xc_booking::domain::slot::slot_rule::SlotRule;
use xc_kernel::ids::BranchId;

const BANGKOK: &str = "Asia/Bangkok";

fn monday() -> NaiveDate {
    let d = NaiveDate::from_ymd_opt(2030, 1, 7).expect("a date");
    assert_eq!(d.weekday(), Weekday::Mon);
    d
}

fn every_day(open: &str, close: &str) -> String {
    let d = format!(r#"[{{"open":"{open}","close":"{close}"}}]"#);
    format!(r#"{{"mon":{d},"tue":{d},"wed":{d},"thu":{d},"fri":{d},"sat":{d},"sun":{d}}}"#)
}

fn bangkok(d: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
    let tz: chrono_tz::Tz = BANGKOK.parse().expect("a timezone");
    tz.from_local_datetime(&d.and_hms_opt(hour, minute, 0).expect("a time"))
        .single()
        .expect("one instant")
        .with_timezone(&Utc)
}

async fn book(db: &TestDb, b: &BranchId, start: DateTime<Utc>) {
    let rows = db.slots(b).await;
    let id = rows
        .iter()
        .find(|r| r.2 == start)
        .unwrap_or_else(|| panic!("slot starting {start}"))
        .0;
    let repo = db.repo();
    let mut tx = db.pool.begin().await.expect("begin");
    repo.reserve_in(&mut tx, id).await.expect("reserve");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn ruling_2_slot_length_change_reshapes_empty_slots_in_place() {
    let db = TestDb::start().await;
    let b = db
        .add_branch(BANGKOK, Some(&every_day("09:00", "11:00")))
        .await;
    let admin = db.add_user().await;
    let service = db.service();

    // 1. Initial 30-minute rule: 4 slots for 1 day.
    service
        .set_rule(
            &b,
            &SlotRule {
                slot_minutes: 30,
                capacity: 3,
                horizon_days: 14,
            },
            admin,
        )
        .await
        .expect("set 30-min rule");

    let initial = service.generate(&b, monday(), 1).await.expect("generate");
    assert_eq!(
        initial,
        Generated {
            added: 4,
            closed: 0,
        }
    );

    let rows_before = db.slots(&b).await;
    assert_eq!(rows_before.len(), 4);
    for r in &rows_before {
        assert_eq!(r.6, "OPEN");
        assert_eq!(r.4, 3); // capacity 3
    }

    // 2. Change rule to 60 minutes, capacity 5.
    service
        .set_rule(
            &b,
            &SlotRule {
                slot_minutes: 60,
                capacity: 5,
                horizon_days: 14,
            },
            admin,
        )
        .await
        .expect("set 60-min rule");

    let got = service
        .generate(&b, monday(), 1)
        .await
        .expect("generate 60-min");
    assert_eq!(
        got,
        Generated {
            added: 0,
            closed: 2,
        },
        "09:30 and 10:30 slots closed; 09:00 and 10:00 reshaped in place"
    );

    let rows_after = db.slots(&b).await;
    assert_eq!(rows_after.len(), 4, "nothing is deleted");

    // Exactly two OPEN one-hour slots: 09:00–10:00 and 10:00–11:00.
    let mut open_slots: Vec<&SlotRow> = rows_after.iter().filter(|r| r.6 == "OPEN").collect();
    open_slots.sort_by_key(|r| r.2);
    assert_eq!(open_slots.len(), 2, "exactly two OPEN slots");

    assert_eq!(open_slots[0].2, bangkok(monday(), 9, 0));
    assert_eq!(open_slots[0].3, bangkok(monday(), 10, 0));
    assert_eq!(open_slots[0].4, 5); // capacity updated to 5
    assert_eq!(open_slots[0].5, 0); // reserved 0

    assert_eq!(open_slots[1].2, bangkok(monday(), 10, 0));
    assert_eq!(open_slots[1].3, bangkok(monday(), 11, 0));
    assert_eq!(open_slots[1].4, 5); // capacity updated to 5
    assert_eq!(open_slots[1].5, 0); // reserved 0

    // The other two slots (09:30 and 10:30) are CLOSED.
    let closed_slots: Vec<&SlotRow> = rows_after.iter().filter(|r| r.6 == "CLOSED").collect();
    assert_eq!(closed_slots.len(), 2);
    assert!(closed_slots.iter().any(|r| r.2 == bangkok(monday(), 9, 30)));
    assert!(
        closed_slots
            .iter()
            .any(|r| r.2 == bangkok(monday(), 10, 30))
    );

    db.finish().await;
}

#[tokio::test]
async fn ruling_2_slot_length_change_leaves_booked_slot_with_its_old_shape() {
    let db = TestDb::start().await;
    let b = db
        .add_branch(BANGKOK, Some(&every_day("09:00", "11:00")))
        .await;
    let admin = db.add_user().await;
    let service = db.service();

    // 1. Initial 30-minute rule.
    service
        .set_rule(
            &b,
            &SlotRule {
                slot_minutes: 30,
                capacity: 3,
                horizon_days: 14,
            },
            admin,
        )
        .await
        .expect("set 30-min rule");

    service.generate(&b, monday(), 1).await.expect("generate");

    // Book one place in the 09:00-09:30 slot.
    let booked_start = bangkok(monday(), 9, 0);
    book(&db, &b, booked_start).await;

    // 2. Change rule to 60 minutes, capacity 5.
    service
        .set_rule(
            &b,
            &SlotRule {
                slot_minutes: 60,
                capacity: 5,
                horizon_days: 14,
            },
            admin,
        )
        .await
        .expect("set 60-min rule");

    let got = service.generate(&b, monday(), 1).await.expect("generate");
    assert_eq!(
        got,
        Generated {
            added: 0,
            closed: 2,
        }
    );

    let rows = db.slots(&b).await;
    assert_eq!(rows.len(), 4, "nothing is deleted");

    // The booked slot kept its old shape: 09:00–09:30, capacity 3, reserved 1, OPEN.
    let booked_slot = rows
        .iter()
        .find(|r| r.2 == booked_start)
        .expect("booked slot");
    assert_eq!(booked_slot.3, bangkok(monday(), 9, 30), "kept old end_at");
    assert_eq!(booked_slot.4, 3, "kept old capacity");
    assert_eq!(booked_slot.5, 1, "kept reserved place");
    assert_eq!(booked_slot.6, "OPEN", "remains OPEN");

    // The 10:00 slot was empty, so it reshaped: 10:00–11:00, capacity 5, reserved 0, OPEN.
    let reshaped_slot = rows
        .iter()
        .find(|r| r.2 == bangkok(monday(), 10, 0))
        .expect("10:00 slot");
    assert_eq!(
        reshaped_slot.3,
        bangkok(monday(), 11, 0),
        "reshaped to 60 min"
    );
    assert_eq!(reshaped_slot.4, 5, "updated capacity");
    assert_eq!(reshaped_slot.5, 0, "reserved 0");
    assert_eq!(reshaped_slot.6, "OPEN");

    // 09:30 and 10:30 are CLOSED.
    let closed: Vec<&SlotRow> = rows.iter().filter(|r| r.6 == "CLOSED").collect();
    assert_eq!(closed.len(), 2);

    db.finish().await;
}

#[tokio::test]
async fn ruling_teller_close_survives_regenerate_with_no_change() {
    let db = TestDb::start().await;
    let b = db
        .add_branch(BANGKOK, Some(&every_day("09:00", "11:00")))
        .await;
    let admin = db.add_user().await;
    let service = db.service();

    // 1. Initial 30-minute rule: 4 slots for 1 day.
    service
        .set_rule(
            &b,
            &SlotRule {
                slot_minutes: 30,
                capacity: 3,
                horizon_days: 14,
            },
            admin,
        )
        .await
        .expect("set 30-min rule");

    let initial = service.generate(&b, monday(), 1).await.expect("generate");
    assert_eq!(
        initial,
        Generated {
            added: 4,
            closed: 0,
        }
    );

    let rows = db.slots(&b).await;
    let slot_0900 = rows
        .iter()
        .find(|r| r.2 == bangkok(monday(), 9, 0))
        .expect("09:00 slot");
    assert_eq!(slot_0900.6, "OPEN");

    // Close the 09:00 slot (empty slot closed by teller).
    service.close(slot_0900.0).await.expect("close empty slot");

    let closed_row = db.slot_row(slot_0900.0).await;
    assert_eq!(closed_row.2, "CLOSED");

    // Re-run generate with no rule change.
    let re_gen = service
        .generate(&b, monday(), 1)
        .await
        .expect("re-generate");
    assert_eq!(
        re_gen,
        Generated {
            added: 0,
            closed: 0,
        }
    );

    // The teller-closed slot must still be CLOSED with unchanged shape.
    let row_after = db.slot_row(slot_0900.0).await;
    assert_eq!(row_after.0, 3, "capacity unchanged");
    assert_eq!(row_after.1, 0, "reserved 0");
    assert_eq!(row_after.2, "CLOSED", "remains CLOSED after re-generate");

    db.finish().await;
}

#[tokio::test]
async fn ruling_teller_close_reshapes_on_slot_minutes_change_and_stays_closed() {
    let db = TestDb::start().await;
    let b = db
        .add_branch(BANGKOK, Some(&every_day("09:00", "11:00")))
        .await;
    let admin = db.add_user().await;
    let service = db.service();

    // 1. Initial 30-minute rule: 4 slots for 1 day.
    service
        .set_rule(
            &b,
            &SlotRule {
                slot_minutes: 30,
                capacity: 3,
                horizon_days: 14,
            },
            admin,
        )
        .await
        .expect("set 30-min rule");

    service.generate(&b, monday(), 1).await.expect("generate");

    let rows = db.slots(&b).await;
    let slot_0900 = rows
        .iter()
        .find(|r| r.2 == bangkok(monday(), 9, 0))
        .expect("09:00 slot");

    // Teller closes the 09:00 slot.
    service.close(slot_0900.0).await.expect("close empty slot");

    // 2. Change rule to 60 minutes, capacity 5.
    service
        .set_rule(
            &b,
            &SlotRule {
                slot_minutes: 60,
                capacity: 5,
                horizon_days: 14,
            },
            admin,
        )
        .await
        .expect("set 60-min rule");

    let got = service
        .generate(&b, monday(), 1)
        .await
        .expect("generate 60-min");
    assert_eq!(
        got,
        Generated {
            added: 0,
            closed: 2,
        }
    );

    let rows_after = db.slots(&b).await;
    let slot_0900_after = rows_after
        .iter()
        .find(|r| r.0 == slot_0900.0)
        .expect("09:00 slot");

    // Reshaped to new end_at and capacity, but status STAYS CLOSED!
    assert_eq!(
        slot_0900_after.3,
        bangkok(monday(), 10, 0),
        "end_at updated to new shape (60m)"
    );
    assert_eq!(slot_0900_after.4, 5, "capacity updated to 5");
    assert_eq!(slot_0900_after.5, 0, "reserved 0");
    assert_eq!(
        slot_0900_after.6, "CLOSED",
        "remains CLOSED despite reshape"
    );

    // 10:00 was OPEN, reshapes and stays OPEN.
    let slot_1000 = rows_after
        .iter()
        .find(|r| r.2 == bangkok(monday(), 10, 0))
        .expect("10:00 slot");
    assert_eq!(slot_1000.3, bangkok(monday(), 11, 0));
    assert_eq!(slot_1000.4, 5);
    assert_eq!(slot_1000.6, "OPEN");

    db.finish().await;
}

#[tokio::test]
async fn ruling_past_empty_slot_keeps_its_old_shape() {
    let db = TestDb::start().await;
    let b = db
        .add_branch(BANGKOK, Some(&every_day("09:00", "11:00")))
        .await;
    let admin = db.add_user().await;
    let service = db.service();

    // A date in the past: 2020-01-06 (Monday).
    let past_monday = NaiveDate::from_ymd_opt(2020, 1, 6).expect("past date");
    assert_eq!(past_monday.weekday(), Weekday::Mon);
    let past_start = bangkok(past_monday, 9, 0);

    // Create a 30-minute slot in the past directly in DB with capacity 3, reserved 0.
    let past_slot_id = db.add_slot(&b, past_start, 30, 3, 0).await;

    // Set rule with 60 minutes and capacity 5.
    service
        .set_rule(
            &b,
            &SlotRule {
                slot_minutes: 60,
                capacity: 5,
                horizon_days: 14,
            },
            admin,
        )
        .await
        .expect("set 60-min rule");

    // Run generate covering the past day.
    service
        .generate(&b, past_monday, 1)
        .await
        .expect("generate");

    // The past slot must keep its original shape: 09:00–09:30, capacity 3, reserved 0, OPEN.
    let row = db.slot_row(past_slot_id).await;
    assert_eq!(row.0, 3, "capacity kept old shape");
    assert_eq!(row.1, 0, "reserved 0");
    assert_eq!(row.2, "OPEN", "status kept OPEN");

    let rows = db.slots(&b).await;
    let past_slot = rows
        .iter()
        .find(|r| r.0 == past_slot_id)
        .expect("past slot");
    assert_eq!(
        past_slot.3,
        bangkok(past_monday, 9, 30),
        "end_at kept old 30-min shape"
    );

    db.finish().await;
}
