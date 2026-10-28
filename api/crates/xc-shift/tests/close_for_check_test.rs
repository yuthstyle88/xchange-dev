//! Staff corrections §4 A, rows A3–A10 (`docs/design/staff-corrections.md`): `CloseFor::check`, pure, no I/O.
//!
//! Written before the implementation by the sub-controller (SC-0, claude/studio-claude-4). Do not edit, delete or
//! weaken these tests; add your own in another file.
//!
//! The branch is on `Asia/Bangkok` (UTC+7). `now` is 2026-10-28 00:30 ICT = 2026-10-27 17:30 UTC, so "today" in
//! the branch is 2026-10-28 while UTC is still on 2026-10-27: the timezone edge of SC-D4.

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::value_objects::close_for::{CloseFor, CloseForRefusal};
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;

const BANGKOK: Tz = chrono_tz::Asia::Bangkok;
const MANAGER: &str = "manager-1";
const TELLER: &str = "teller-1";

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
        .single()
        .expect("a valid time")
}

/// 2026-10-28 00:30 ICT.
fn now() -> DateTime<Utc> {
    utc(2026, 10, 27, 17, 30)
}

/// The teller's clock-in on 2026-10-26 09:00 ICT (02:00 UTC), still open.
fn open_row() -> Attendance {
    Attendance {
        id: "att-1".to_owned(),
        employee_id: TELLER.to_owned(),
        role: CustodyRole::Front,
        clock_in: CustodyTime::from_epoch_millis(utc(2026, 10, 26, 2, 0).timestamp_millis()),
        clock_out: None,
    }
}

fn close_at(clock_out: DateTime<Utc>, reason: &str) -> CloseFor {
    CloseFor {
        attendance_id: "att-1".to_owned(),
        clock_out,
        reason: reason.to_owned(),
    }
}

/// 2026-10-26 18:00 ICT: after the clock-in, two days back.
fn good_time() -> DateTime<Utc> {
    utc(2026, 10, 26, 11, 0)
}

#[test]
fn a_good_close_passes() {
    assert_eq!(
        close_at(good_time(), "forgot to clock out").check(&open_row(), MANAGER, now(), BANGKOK),
        Ok(())
    );
}

// A3
#[test]
fn closing_your_own_row_is_self_close() {
    let mut row = open_row();
    row.employee_id = MANAGER.to_owned();
    assert_eq!(
        close_at(good_time(), "forgot").check(&row, MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::SelfClose)
    );
}

// A4
#[test]
fn a_closed_row_is_not_open() {
    let mut row = open_row();
    row.clock_out = Some(CustodyTime::from_epoch_millis(
        utc(2026, 10, 26, 10, 0).timestamp_millis(),
    ));
    assert_eq!(
        close_at(good_time(), "forgot").check(&row, MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::NotOpen)
    );
}

// A5
#[test]
fn a_blank_reason_is_required() {
    for reason in ["", "   ", "\t\n"] {
        assert_eq!(
            close_at(good_time(), reason).check(&open_row(), MANAGER, now(), BANGKOK),
            Err(CloseForRefusal::ReasonRequired),
            "reason {reason:?}"
        );
    }
}

// A6 (SC-D10): 500 UTF-16 units pass, 501 do not; a Thai character is one unit.
#[test]
fn a_reason_longer_than_500_units_is_too_long() {
    let five_hundred = "ก".repeat(500);
    assert_eq!(
        close_at(good_time(), &five_hundred).check(&open_row(), MANAGER, now(), BANGKOK),
        Ok(())
    );
    let five_hundred_one = "ก".repeat(501);
    assert_eq!(
        close_at(good_time(), &five_hundred_one).check(&open_row(), MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::ReasonTooLong)
    );
}

// A8
#[test]
fn a_clock_out_before_the_clock_in_is_before_clock_in() {
    assert_eq!(
        close_at(utc(2026, 10, 26, 1, 59), "forgot").check(&open_row(), MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::BeforeClockIn)
    );
    // equal to the clock-in is allowed (`clockOut ≥ clockIn`)
    assert_eq!(
        close_at(utc(2026, 10, 26, 2, 0), "forgot").check(&open_row(), MANAGER, now(), BANGKOK),
        Ok(())
    );
}

// A9
#[test]
fn a_clock_out_after_now_is_in_future() {
    assert_eq!(
        close_at(utc(2026, 10, 27, 17, 31), "forgot").check(&open_row(), MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::InFuture)
    );
}

// A10 and the timezone edge: yesterday 23:30 ICT passes; today 00:10 ICT is not an earlier day, although its UTC
// date (2026-10-27) is before the UTC date of now's ICT day.
#[test]
fn the_earlier_day_is_decided_in_the_branch_timezone() {
    // 2026-10-27 23:30 ICT = 16:30 UTC
    assert_eq!(
        close_at(utc(2026, 10, 27, 16, 30), "forgot").check(&open_row(), MANAGER, now(), BANGKOK),
        Ok(())
    );
    // 2026-10-28 00:10 ICT = 2026-10-27 17:10 UTC
    assert_eq!(
        close_at(utc(2026, 10, 27, 17, 10), "forgot").check(&open_row(), MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::NotEarlierDay)
    );
    // a branch on UTC: today there is 2026-10-27, the instant's own date, so it is refused there too
    assert_eq!(
        close_at(utc(2026, 10, 27, 17, 10), "forgot").check(
            &open_row(),
            MANAGER,
            now(),
            chrono_tz::UTC
        ),
        Err(CloseForRefusal::NotEarlierDay)
    );
    // and 2026-10-26 23:00 UTC is an earlier day in UTC but today-minus-one in ICT too: passes in both
    for tz in [BANGKOK, chrono_tz::UTC] {
        assert_eq!(
            close_at(utc(2026, 10, 26, 16, 0), "forgot").check(&open_row(), MANAGER, now(), tz),
            Ok(()),
            "{tz}"
        );
    }
}

// A10: a clock-out on the earlier day is refused when the branch's day differs from UTC's. A branch on
// America/Los_Angeles (UTC−7 in October): now is 2026-10-27 10:30 there, so 2026-10-27 01:00 local is today.
#[test]
fn a_branch_behind_utc_uses_its_own_today() {
    let la: Tz = chrono_tz::America::Los_Angeles;
    // 2026-10-27 01:00 PDT = 08:00 UTC; after the clock-in, before now, but today in LA
    assert_eq!(
        close_at(utc(2026, 10, 27, 8, 0), "forgot").check(&open_row(), MANAGER, now(), la),
        Err(CloseForRefusal::NotEarlierDay)
    );
    // 2026-10-26 23:00 PDT = 2026-10-27 06:00 UTC: yesterday in LA
    assert_eq!(
        close_at(utc(2026, 10, 27, 6, 0), "forgot").check(&open_row(), MANAGER, now(), la),
        Ok(())
    );
}

// The order of A3–A10: the first fault wins.
#[test]
fn the_first_fault_wins() {
    let mut own_closed = open_row();
    own_closed.employee_id = MANAGER.to_owned();
    own_closed.clock_out = Some(CustodyTime::from_epoch_millis(0));
    // self before not-open
    assert_eq!(
        close_at(good_time(), "").check(&own_closed, MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::SelfClose)
    );
    let mut closed = open_row();
    closed.clock_out = Some(CustodyTime::from_epoch_millis(0));
    // not-open before reason
    assert_eq!(
        close_at(utc(2030, 1, 1, 0, 0), "").check(&closed, MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::NotOpen)
    );
    // reason before the time rules
    assert_eq!(
        close_at(utc(2030, 1, 1, 0, 0), " ").check(&open_row(), MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::ReasonRequired)
    );
    // before-clock-in before in-future cannot both hold; in-future before not-earlier-day
    assert_eq!(
        close_at(utc(2030, 1, 1, 0, 0), "forgot").check(&open_row(), MANAGER, now(), BANGKOK),
        Err(CloseForRefusal::InFuture)
    );
}

#[test]
fn each_refusal_has_its_code_and_field() {
    let table = [
        (CloseForRefusal::SelfClose, "selfClose", None),
        (CloseForRefusal::NotOpen, "notOpen", None),
        (
            CloseForRefusal::ReasonRequired,
            "reasonRequired",
            Some("reason"),
        ),
        (
            CloseForRefusal::ReasonTooLong,
            "validationFailed",
            Some("reason"),
        ),
        (
            CloseForRefusal::BeforeClockIn,
            "beforeClockIn",
            Some("clockOut"),
        ),
        (CloseForRefusal::InFuture, "inFuture", Some("clockOut")),
        (
            CloseForRefusal::NotEarlierDay,
            "notEarlierDay",
            Some("clockOut"),
        ),
    ];
    for (refusal, code, field) in table {
        assert_eq!(refusal.code(), code, "{refusal:?}");
        assert_eq!(refusal.field(), field, "{refusal:?}");
    }
}
