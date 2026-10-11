//! QU-7 (#1266): the public `branch_service_status` view (design `queue-tickets.md` §4.4, Q14,
//! C08). It carries the queue's state and the wait estimate, and no customer field.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// WAITING tickets per service type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_service_status.ts")]
pub struct BranchServiceWaiting {
    pub walk_in: i32,
    pub booking: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_service_status.ts")]
pub struct BranchServiceStatusData {
    pub branch: String,
    pub queue_open: bool,
    pub active_counters: i32,
    pub waiting: BranchServiceWaiting,
    /// The wait a customer joining now can expect, with its range (Q-A5, Q-A6); `null` together
    /// when there is nothing to estimate from.
    pub estimate_seconds: Option<i32>,
    pub estimate_low_seconds: Option<i32>,
    pub estimate_high_seconds: Option<i32>,
    pub estimate_as_of: Option<String>,
}
