//! A shift the Owner configures for a branch (configurable-shifts.md §2.1).

use chrono::NaiveTime;

/// A live `shift_templates` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftTemplate {
    pub id: String,
    pub name: String,
    pub starts_at: NaiveTime,
    /// `ends_at <= starts_at` means the shift ends on the next day.
    pub ends_at: NaiveTime,
    /// `(start, end)` wall-clock times; `end <= start` means the break ends on the next day.
    pub break_window: Option<(NaiveTime, NaiveTime)>,
    pub position: i16,
}

/// One row of the Owner's list as sent to `branchShiftsSave`. A row without `id` is new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftTemplateInput {
    pub id: Option<String>,
    pub name: String,
    pub starts_at: NaiveTime,
    pub ends_at: NaiveTime,
    pub break_window: Option<(NaiveTime, NaiveTime)>,
}
