//! Backdated keying (`docs/design/backdated-keying.md` §5, §9): who really keyed a backdated exchange, and when.
//! Read back from `transactions.recorded_at` / `recorded_by` of a row with `backdated = true`.

use chrono::{DateTime, Utc};

/// `[required]` The real entry of a backdated exchange (owner rule R2: stored separately from the deal time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackdateMark {
    /// The real time the row was keyed (`transactions.recorded_at`).
    pub entered_at: DateTime<Utc>,
    /// The user who keyed it (`transactions.recorded_by`).
    pub entered_by: String,
}
