//! Member entity.

use chrono::{DateTime, Utc};
use xc_kernel::ids::BranchId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub branch_id: BranchId,
    pub customer_id: String,
    pub name: String,
    pub phone: String,
    pub token_hash: String,
    pub verified_at: DateTime<Utc>,
    pub verified_by: String,
}
