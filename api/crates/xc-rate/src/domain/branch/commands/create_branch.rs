//! Create branch command.

use xc_kernel::ids::BranchId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBranch {
    pub request_id: String,
    pub name: String,
    pub from_branch: BranchId,
}
