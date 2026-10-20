use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use xc_kernel::ids::BranchId;

use super::role::Role;

/// One `user_roles` row. RBAC §11.4: `role` is the group's **base** (every rule that matches on a role reads it:
/// `GrantGuard`, `ScopeRule`, the last-owner rule, the desk checks), and `group` is the group's code (what it may do:
/// `RoleGrants::holds(group, p)`). A built-in group's code is its base's code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedRole {
    pub role: Role,
    /// The group code (`roles.code`). Empty only in a row serialised before §11; read it through [`Self::group_code`].
    #[serde(default)]
    pub group: String,
    pub branch: Option<BranchId>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

impl ScopedRole {
    /// A row of the built-in group `role`: its code is `role`'s and its base is `role`.
    pub fn new(
        role: Role,
        branch: Option<BranchId>,
        valid_from: DateTime<Utc>,
        valid_to: Option<DateTime<Utc>>,
    ) -> Self {
        Self::in_group(role.as_str(), role, branch, valid_from, valid_to)
    }

    /// A row of the group `group`, whose base is `base` (RBAC §11.1).
    pub fn in_group(
        group: &str,
        base: Role,
        branch: Option<BranchId>,
        valid_from: DateTime<Utc>,
        valid_to: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            role: base,
            group: group.to_owned(),
            branch,
            valid_from,
            valid_to,
        }
    }

    /// The group code this row grants through; a row without one is its base's built-in group.
    pub fn group_code(&self) -> &str {
        if self.group.is_empty() {
            self.role.as_str()
        } else {
            &self.group
        }
    }

    pub fn covers(&self, branch: Option<&str>) -> bool {
        match &self.branch {
            None => true,
            Some(own) => branch == Some(own.as_str()),
        }
    }
}
