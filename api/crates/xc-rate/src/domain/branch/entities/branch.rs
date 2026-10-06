//! Branch entity.

use xc_kernel::ids::BranchId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub id: BranchId,
    pub name: String,
    pub request_id: Option<String>,
}
