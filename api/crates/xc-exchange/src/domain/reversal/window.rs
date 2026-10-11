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

    /// True while the date of `now` in `tz` is not after `business_date`.
    pub fn is_open(business_date: NaiveDate, now: DateTime<Utc>, tz: Tz) -> bool {
        now.with_timezone(&tz).date_naive() <= business_date
    }
}
