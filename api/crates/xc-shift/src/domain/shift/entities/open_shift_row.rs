//! One OPEN shift of a branch with its teller (CA-10p, design `cash-shift-transfer.md` §4.7):
//! what `ShiftRepository::open_of_branch` answers for the admin views' `openShifts`.

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenShiftRow {
    pub shift_id: String,
    /// `shifts.teller_id` as text (`users.id`).
    pub teller_id: String,
    /// `users.name` of that teller.
    pub teller_name: String,
    pub opened_at: DateTime<Utc>,
}
