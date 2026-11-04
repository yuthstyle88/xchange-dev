//! Backdated keying (`docs/design/backdated-keying.md` §5, behaviour table B5, B6, B16 and the guard G):
//! `BackdateWindow::check` and `BackdateRefusal`'s code and field.
//! Written before the implementation by the sub-controller claude/6bb512ce. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Restated for CS-20b by the configurable-shifts sub-controller claude/35271802 (lane controller's ruling on #564,
//! issuecomment-6633086725: one business-date rule; a backdated row's `value_date` is `ShiftPlan::business_date_of`
//! its `paid_at`). `check` now takes the two business dates the caller reads from the plan instead of a timezone:
//! `paid_day` for `paid_at` and `today` for `now`. W1–W4 and W6–W9 pass the Bangkok calendar date, which the default
//! plan gives for any time from 06:00 on; W5 and W10 pin the night hours before the 06:00 cutover.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Asia::Bangkok;
use xc_exchange::domain::transaction::backdate_window::{BackdateRefusal, BackdateWindow};

/// 2026-11-04 20:30 ICT, when the owner ordered the feature.
fn now() -> DateTime<Utc> {
    Bangkok
        .with_ymd_and_hms(2026, 11, 4, 20, 30, 0)
        .unwrap()
        .with_timezone(&Utc)
}

fn ict(d: u32, h: u32, m: u32) -> DateTime<Utc> {
    Bangkok
        .with_ymd_and_hms(2026, 11, d, h, m, 0)
        .unwrap()
        .with_timezone(&Utc)
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

/// The Bangkok calendar date of `t`: the business date the default plan (cutover 06:00) gives from 06:00 to 24:00.
fn cal(t: DateTime<Utc>) -> NaiveDate {
    t.with_timezone(&Bangkok).date_naive()
}

/// B16: a deal earlier today, on today's business date, not closed, is allowed.
#[test]
fn w1_today_earlier_and_not_closed_is_allowed() {
    assert_eq!(
        BackdateWindow::check(ict(4, 13, 23), day(4), now(), cal(ict(4, 13, 23)), cal(now()), false),
        Ok(())
    );
}

/// A deal on an earlier day, on that day's business date, not closed, is allowed.
#[test]
fn w2_an_earlier_day_not_closed_is_allowed() {
    assert_eq!(
        BackdateWindow::check(ict(2, 9, 0), day(2), now(), cal(ict(2, 9, 0)), cal(now()), false),
        Ok(())
    );
}

/// B5: `paid_at` one minute after now is refused, whatever else holds.
#[test]
fn w3_paid_at_in_the_future_is_refused_first() {
    let later = now() + Duration::minutes(1);
    assert_eq!(
        BackdateWindow::check(later, day(4), now(), cal(later), cal(now()), true),
        Err(BackdateRefusal::PaidAtInFuture)
    );
}

/// `paid_at` equal to now is not the future.
#[test]
fn w4_paid_at_equal_to_now_is_allowed() {
    assert_eq!(
        BackdateWindow::check(now(), day(4), now(), cal(now()), cal(now()), false),
        Ok(())
    );
}

/// B6 restated (CS-20b): 2026-11-04 00:30 ICT is before the 06:00 cutover, so the plan dates it 2026-11-03; the
/// value date must be that business date, not the Bangkok calendar date.
#[test]
fn w5_value_date_must_be_the_business_date_of_paid_at() {
    let paid = ict(4, 0, 30);
    assert_eq!(
        BackdateWindow::check(paid, day(4), now(), day(3), cal(now()), false),
        Err(BackdateRefusal::ValueDateNotPaidDate)
    );
    assert_eq!(
        BackdateWindow::check(paid, day(3), now(), day(3), cal(now()), false),
        Ok(())
    );
}

/// A business date after today is refused (checked after the paid-date match, so the two disagree here).
#[test]
fn w6_value_date_after_today_is_refused() {
    assert_eq!(
        BackdateWindow::check(ict(4, 10, 0), day(5), now(), cal(ict(4, 10, 0)), cal(now()), false),
        Err(BackdateRefusal::ValueDateNotPaidDate)
    );
    // At 2026-11-04 23:59 ICT now, a value date of today is fine; tomorrow can never match a past paid_at.
    let late = ict(4, 23, 59);
    assert_eq!(
        BackdateWindow::check(late, day(4), late, cal(late), cal(late), false),
        Ok(())
    );
}

/// Guard G: a closed business date is refused, last.
#[test]
fn w7_a_closed_day_is_refused() {
    assert_eq!(
        BackdateWindow::check(ict(2, 9, 0), day(2), now(), cal(ict(2, 9, 0)), cal(now()), true),
        Err(BackdateRefusal::DayClosed)
    );
}

/// The order: a wrong date wins over a closed day.
#[test]
fn w8_a_wrong_date_is_named_before_a_closed_day() {
    assert_eq!(
        BackdateWindow::check(ict(2, 9, 0), day(1), now(), cal(ict(2, 9, 0)), cal(now()), true),
        Err(BackdateRefusal::ValueDateNotPaidDate)
    );
}

/// §4: the codes and fields the route answers with.
#[test]
fn w9_codes_and_fields() {
    use BackdateRefusal::*;
    assert_eq!(PaidAtInFuture.code(), "validationFailed");
    assert_eq!(PaidAtInFuture.field(), Some("payment.paidAt"));
    assert_eq!(ValueDateNotPaidDate.code(), "validationFailed");
    assert_eq!(ValueDateNotPaidDate.field(), Some("valueDate"));
    assert_eq!(ValueDateInFuture.code(), "validationFailed");
    assert_eq!(ValueDateInFuture.field(), Some("valueDate"));
    assert_eq!(DayClosed.code(), "dayClosed");
    assert_eq!(DayClosed.field(), None);
}

/// W10 (CS-20b, the ruling's 02:00 row): at 2026-11-05 03:00 ICT the business date is still 2026-11-04, so a deal
/// paid at 02:00 on 2026-11-05 is dated 2026-11-04 and is not in the future; the calendar date 2026-11-05 is refused.
#[test]
fn w10_a_two_am_deal_belongs_to_the_previous_business_date() {
    let now = ict(5, 3, 0);
    let paid = ict(5, 2, 0);
    assert_eq!(
        BackdateWindow::check(paid, day(4), now, day(4), day(4), false),
        Ok(())
    );
    assert_eq!(
        BackdateWindow::check(paid, day(5), now, day(4), day(4), false),
        Err(BackdateRefusal::ValueDateNotPaidDate)
    );
}
