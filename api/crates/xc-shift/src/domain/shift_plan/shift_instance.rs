//! One template on one business date, with real instants (configurable-shifts.md §2.2 T3).

use chrono::{DateTime, NaiveDate, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftInstance {
    pub template_id: String,
    pub name: String,
    pub business_date: NaiveDate,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub break_window: Option<(DateTime<Utc>, DateTime<Utc>)>,
}
