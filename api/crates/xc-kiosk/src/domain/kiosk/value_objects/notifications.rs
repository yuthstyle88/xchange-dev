//! What the desk's bell shows.

use crate::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use crate::domain::kiosk::value_objects::kiosk_time::KioskTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingRequest {
    pub id: String,
    pub staff: String,
    pub started_at: Option<KioskTime>,
    pub status: KioskStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notifications {
    pub branch_name: String,
    /// Every waiting request of the branch.
    pub count: i64,
    /// The waiting request whose quote is the newest.
    pub latest: Option<String>,
    /// Requests currently at work (ACCEPTED / ID_CHECKED), up to 10.
    pub working: Vec<WorkingRequest>,
    /// Total count of requests currently at work.
    pub working_count: i64,
}
