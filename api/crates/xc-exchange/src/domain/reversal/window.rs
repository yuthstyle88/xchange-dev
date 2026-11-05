//! R-1: a reversal is requested and posted only within the original's business day, in the branch timezone
//! (design `reversal.md` §0 R-1, §4.1).

use chrono::{DateTime, NaiveDate, Utc};
pub use chrono_tz::Tz;

pub struct ReversalWindow;

impl ReversalWindow {
    /// The date of `completed_at` in `tz`: the original's business date.
    pub fn business_date_of(completed_at: DateTime<Utc>, tz: Tz) -> NaiveDate {
        completed_at.with_timezone(&tz).date_naive()
    }

    /// `[required]` CS-20 (design §2.10): true while `now` is before `deadline`, the end of the original's business day
    /// (`ShiftPlan::business_day_end(value_date)`, computed by the caller: xc-exchange does not depend on xc-shift).
    /// Replaces `business_date_of` and `is_open`, which CS-20 deletes once no caller is left.
    pub fn is_open_until(deadline: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        let _ = (deadline, now);
        todo!("CS-20")
    }

    /// True while the date of `now` in `tz` is not after `business_date`.
    pub fn is_open(business_date: NaiveDate, now: DateTime<Utc>, tz: Tz) -> bool {
        now.with_timezone(&tz).date_naive() <= business_date
    }
}
