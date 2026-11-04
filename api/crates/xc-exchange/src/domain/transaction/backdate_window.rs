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
        todo!("BK-2: backdated-keying.md §4")
    }

    /// `[required]` The input field the refusal names: `"payment.paidAt"` for [`Self::PaidAtInFuture`],
    /// `"valueDate"` for the two date refusals, `None` for [`Self::DayClosed`].
    pub fn field(self) -> Option<&'static str> {
        todo!("BK-2: backdated-keying.md §4")
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
        let _ = (paid_at, value_date, now, tz, closed);
        todo!("BK-2: backdated-keying.md §5")
    }
}
