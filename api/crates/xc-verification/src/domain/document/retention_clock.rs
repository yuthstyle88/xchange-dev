//! `RetentionClock` (design V13): when an ended case's document may be purged.

use chrono::{DateTime, Duration, Utc};

pub struct RetentionClock;

impl RetentionClock {
    /// `ended_at + period`; `None` when the type has no period (never purged, U2).
    pub fn retention_until(
        ended_at: DateTime<Utc>,
        period: Option<Duration>,
    ) -> Option<DateTime<Utc>> {
        period.map(|p| ended_at + p)
    }
}
