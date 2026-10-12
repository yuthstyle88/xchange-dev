//! `BookingSlot` (design §5, §4.1 `booking_slots`): one bookable time of a branch with its capacity.

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;
use xc_kernel::ids::BranchId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotStatus {
    Open,
    Closed,
}

impl SlotStatus {
    /// The `booking_slots.status` text: `OPEN` or `CLOSED`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Closed => "CLOSED",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookingSlot {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    /// The branch's local date of `start_at` (§4.1).
    pub service_date: NaiveDate,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub capacity: i32,
    /// The slot's bookings in a holding state (§4.1); `0 ≤ reserved ≤ capacity`.
    pub reserved: i32,
    pub status: SlotStatus,
}
