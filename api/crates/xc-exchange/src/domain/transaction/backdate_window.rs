//! Backdated keying (`docs/design/backdated-keying.md` §5; owner rule R2 and guard G): which deal times and
//! business dates a backdated exchange may be keyed at. Pure: the caller passes `now`, the branch's timezone and
//! whether the business date is closed.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;

/// `[required]` Why a deal time or business date is refused, in the order `BackdateWindow::check` asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackdateRefusal {
    /// `paid_at` is later than `now`.
    PaidAtInFuture,
    /// `value_date` is not `paid_at`'s date in the branch's timezone.
    ValueDateNotPaidDate,
    /// `value_date` is after today in the branch's timezone.
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
    /// (`value_date` ≠ `paid_at`'s date in `tz`) → `ValueDateInFuture` (`value_date` > `now`'s date in `tz`) →
    /// `DayClosed` (`closed`). Today counts as a past date.
    pub fn check(
        paid_at: DateTime<Utc>,
        value_date: NaiveDate,
        now: DateTime<Utc>,
        tz: Tz,
        closed: bool,
    ) -> Result<(), BackdateRefusal> {
        if paid_at > now {
            return Err(BackdateRefusal::PaidAtInFuture);
        }
        if value_date != paid_at.with_timezone(&tz).date_naive() {
            return Err(BackdateRefusal::ValueDateNotPaidDate);
        }
        if value_date > now.with_timezone(&tz).date_naive() {
            return Err(BackdateRefusal::ValueDateInFuture);
        }
        if closed {
            return Err(BackdateRefusal::DayClosed);
        }
        Ok(())
    }
}
