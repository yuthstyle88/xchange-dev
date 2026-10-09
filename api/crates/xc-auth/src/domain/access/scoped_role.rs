use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use xc_kernel::ids::BranchId;

use super::role::Role;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedRole {
    pub role: Role,
    pub branch: Option<BranchId>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

impl ScopedRole {
    pub fn new(
        role: Role,
        branch: Option<BranchId>,
        valid_from: DateTime<Utc>,
        valid_to: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            role,
            branch,
            valid_from,
            valid_to,
        }
    }
}
