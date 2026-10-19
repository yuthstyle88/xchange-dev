//! RBAC §11.4 (design `docs/design/blueprint/user-management-rbac.md`): groups are data.
//!
//! - [`RoleGrants::seed`] is the only source of §11.2's seed (the owner's V.9 matrix through the Blueprint's mapping).
//! - [`RoleGrantsSnapshot`] is every group and its permission set, as `roles` + `role_permission_grants` hold them,
//!   stamped with `role_grant_version`.
//! - [`RoleGrants::current`] is the process's snapshot. Until something calls [`RoleGrants::install`] it is
//!   [`RoleGrantsSnapshot::seeded`], so code and tests with no database authorise from the seed.
//!
//! Skeleton (claude/65357afb, `feat/rbac-groups`): bodies are C6a's.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::permission::Permission;
use super::role::Role;

/// One group: a `roles` row (§11.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub code: String,
    pub name: String,
    /// Decides every rule the code keeps by role. A built-in group's base is itself.
    pub base: Role,
    pub built_in: bool,
}

impl Group {
    /// The built-in group of `role`: code and name are `role`'s code, base is `role`.
    pub fn built_in(role: Role) -> Self {
        Self {
            code: role.as_str().to_owned(),
            name: role.as_str().to_owned(),
            base: role,
            built_in: true,
        }
    }
}

/// Every group and its set, at one `role_grant_version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleGrantsSnapshot {
    version: i64,
    groups: BTreeMap<String, (Group, BTreeSet<Permission>)>,
}

impl RoleGrantsSnapshot {
    /// A snapshot of `groups` at `version`. A later entry with the same code replaces an earlier one.
    pub fn new(version: i64, groups: Vec<(Group, Vec<Permission>)>) -> Self {
        let _ = (version, groups);
        todo!("C6a")
    }

    /// Version 0: the nine built-in groups ([`RoleGrants::BUILT_IN`]), each holding [`RoleGrants::seed`] of itself.
    pub fn seeded() -> Self {
        todo!("C6a")
    }

    pub fn version(&self) -> i64 {
        self.version
    }

    pub fn group(&self, code: &str) -> Option<&Group> {
        self.groups.get(code).map(|(g, _)| g)
    }

    /// The group's set; `None` for a code the snapshot does not hold.
    pub fn permissions(&self, code: &str) -> Option<&BTreeSet<Permission>> {
        self.groups.get(code).map(|(_, set)| set)
    }

    /// Every group, by code.
    pub fn groups(&self) -> impl Iterator<Item = (&Group, &BTreeSet<Permission>)> {
        self.groups.values().map(|(g, set)| (g, set))
    }
}

pub struct RoleGrants;

impl RoleGrants {
    /// The built-in groups (`roles` rows before §11), in `Role` declaration order. CUSTOMER is one; it is not shown.
    pub const BUILT_IN: [Role; 9] = [
        Role::Executive,
        Role::Admin,
        Role::RateManager,
        Role::BranchManager,
        Role::Teller,
        Role::Finance,
        Role::Marketing,
        Role::Customer,
        Role::VerificationReviewer,
    ];

    /// §11.2: the seed of the built-in group `role`, in [`Permission::ALL`] order, no duplicates.
    /// - A module level grants every permission of that module at or below it (`PermissionModule::module`).
    /// - The no-module permissions (`PermissionModule::module` = `None`) are what `role` holds today.
    /// - MARKETING and CUSTOMER keep today's sets.
    pub fn seed(role: Role) -> Vec<Permission> {
        let _ = role;
        todo!("C6a")
    }

    /// The process's snapshot.
    pub fn current() -> Arc<RoleGrantsSnapshot> {
        todo!("C6a")
    }

    /// Replaces the process's snapshot. Startup and `read_session_in` call it when `role_grant_version` moved.
    pub fn install(snapshot: RoleGrantsSnapshot) {
        let _ = snapshot;
        todo!("C6a")
    }

    /// Does the group `group` hold `permission` in the current snapshot? An unknown code holds nothing.
    pub fn holds(group: &str, permission: Permission) -> bool {
        let _ = (group, permission);
        todo!("C6a")
    }

    /// The group's set in the current snapshot, in [`Permission::ALL`] order; empty for an unknown code.
    pub fn of(group: &str) -> Vec<Permission> {
        let _ = group;
        todo!("C6a")
    }

    /// The group's base in the current snapshot; a built-in code parses to its role even when the snapshot lacks it.
    pub fn base_of(group: &str) -> Option<Role> {
        let _ = group;
        todo!("C6a")
    }
}
