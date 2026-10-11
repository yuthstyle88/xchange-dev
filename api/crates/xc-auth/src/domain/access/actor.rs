use serde::{Deserialize, Serialize};
use xc_kernel::ids::BranchId;

use super::scoped_role::ScopedRole;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub user_id: String,
    pub organization_id: String,
    pub roles: Vec<ScopedRole>,
    pub customer_id: Option<String>,
    /// The session's effective branch (SB-1 §14.3). `None` from [`Actor::new`]; the session extractor sets it with
    /// [`Actor::with_branch`]. `Authorizer::check` grants it when a request names no branch and a qualifying row
    /// covers it.
    #[serde(default)]
    pub branch: Option<BranchId>,
}

impl Actor {
    pub fn new(
        user_id: &str,
        organization_id: &str,
        roles: Vec<ScopedRole>,
        customer_id: Option<&str>,
    ) -> Self {
        Self {
            user_id: user_id.to_string(),
            organization_id: organization_id.to_string(),
            roles,
            customer_id: customer_id.map(str::to_string),
            branch: None,
        }
    }

    /// The same actor working in `branch` (the session branch).
    pub fn with_branch(mut self, branch: Option<BranchId>) -> Self {
        self.branch = branch;
        self
    }
}
