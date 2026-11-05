//! R-1: a reversal is requested and posted only within the original's business day
//! (design `reversal.md` §0 R-1, §4.1).

use chrono::{DateTime, Utc};
pub use chrono_tz::Tz;

pub struct ReversalWindow;

impl ReversalWindow {
    /// `[required]` CS-20 (design §2.10): true while `now` is before `deadline`, the end of the original's business day
    /// (`ShiftPlan::business_day_end(value_date)`, computed by the caller: xc-exchange does not depend on xc-shift).
    pub fn is_open_until(deadline: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        now < deadline
    }
}
