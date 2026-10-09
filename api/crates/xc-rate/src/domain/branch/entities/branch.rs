//! Branch entity.

use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::branch::branch_hours::BranchHours;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub id: BranchId,
    pub name: String,
    pub request_id: Option<String>,
    pub code: Option<String>,
    pub address: Option<String>,
    pub timezone: String,
    pub opening_hours: Option<serde_json::Value>,
    pub booking_enabled: bool,
}

impl Branch {
    pub fn hours(&self) -> AppResult<BranchHours> {
        BranchHours::new(&self.timezone, self.opening_hours.as_ref())
    }
}
