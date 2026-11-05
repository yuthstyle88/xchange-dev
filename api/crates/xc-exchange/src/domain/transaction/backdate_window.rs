//! Backdated keying (`docs/design/backdated-keying.md` §5; owner rule R2 and guard G): which deal times and
//! business dates a backdated exchange may be keyed at. Pure: the caller passes `now`, the business dates of `paid_at`
//! and of `now` (CS-20b: `ShiftPlan::business_date_of` under the branch's plan; one business-date rule, ruling on
//! #564 issuecomment-6633086725) and whether the business date is closed.

use chrono::{DateTime, NaiveDate, Utc};

/// `[required]` Why a deal time or business date is refused, in the order `BackdateWindow::check` asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackdateRefusal {
    /// `paid_at` is later than `now`.
    PaidAtInFuture,
    /// `value_date` is not `paid_at`'s business date.
    ValueDateNotPaidDate,
    /// `value_date` is after `now`'s business date.
    ValueDateInFuture,
    /// `value_date` is a closed (certified) business date.
    DayClosed,
}

impl BackdateRefusal {
    /// `[required]` The v3 error code: `"dayClosed"` for [`Self::DayClosed`], `"validationFailed"` otherwise.
    pub fn code(self) -> &'static str {
        match self {
            Self::DayClosed => "dayClosed",
            Self::PaidAtInFuture | Self::ValueDateNotPaidDate | Self::ValueDateInFuture => {
                "validationFailed"
            }
        }
    }

    /// `[required]` The input field the refusal names: `"payment.paidAt"` for [`Self::PaidAtInFuture`],
    /// `"valueDate"` for the two date refusals, `None` for [`Self::DayClosed`].
    pub fn field(self) -> Option<&'static str> {
        match self {
            Self::PaidAtInFuture => Some("payment.paidAt"),
            Self::ValueDateNotPaidDate | Self::ValueDateInFuture => Some("valueDate"),
            Self::DayClosed => None,
        }
    }
}

pub struct BackdateWindow;

impl BackdateWindow {
    /// `[required]` Refuses, in this order: `PaidAtInFuture` (`paid_at > now`) → `ValueDateNotPaidDate`
    /// (`value_date` ≠ `paid_day`, the business date of `paid_at`) → `ValueDateInFuture` (`value_date` > `today`, the
    /// business date of `now`) → `DayClosed` (`closed`). Today counts as a past date.
    pub fn check(
        paid_at: DateTime<Utc>,
        value_date: NaiveDate,
        now: DateTime<Utc>,
        paid_day: NaiveDate,
        today: NaiveDate,
        closed: bool,
    ) -> Result<(), BackdateRefusal> {
        if paid_at > now {
            return Err(BackdateRefusal::PaidAtInFuture);
        }
        if value_date != paid_day {
            return Err(BackdateRefusal::ValueDateNotPaidDate);
        }
        if value_date > today {
            return Err(BackdateRefusal::ValueDateInFuture);
        }
        if closed {
            return Err(BackdateRefusal::DayClosed);
        }
        Ok(())
    }
}
