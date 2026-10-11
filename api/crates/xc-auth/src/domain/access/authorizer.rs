use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use xc_kernel::ids::BranchId;

use super::actor::Actor;
use super::permission::Permission;
use super::role::Role;
use super::role_permissions::RolePermissions;
use super::scoped_role::ScopedRole;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub organization_id: String,
    pub branch: Option<BranchId>,
    pub customer_id: Option<String>,
}

impl Target {
    pub fn new(organization_id: &str, branch: Option<BranchId>, customer_id: Option<&str>) -> Self {
        Self {
            organization_id: organization_id.to_string(),
            branch,
            customer_id: customer_id.map(str::to_string),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub branch: BranchId,
    pub roles_used: Vec<Role>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    SignIn,
    OutOfScope,
    RoleNotAllowed,
}

impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::SignIn => "signIn",
            Refusal::OutOfScope => "outOfScope",
            Refusal::RoleNotAllowed => "roleNotAllowed",
        }
    }
}

pub struct Authorizer;

/// The actor's roles that count for `permission` at `now`: valid then (`valid_from <= now < valid_to`) and
/// holding the permission. The one rule `Authorizer::check` and `BranchScope::of` both use (EX-1 #1070).
pub(crate) fn qualifying(
    actor: &Actor,
    permission: Permission,
    now: DateTime<Utc>,
) -> impl Iterator<Item = &ScopedRole> {
    actor.roles.iter().filter(move |r| {
        r.valid_from <= now
            && r.valid_to.is_none_or(|to| now < to)
            && RolePermissions::of(r.role).contains(&permission)
    })
}

impl Authorizer {
    pub fn check(
        actor: Option<&Actor>,
        permission: Permission,
        target: &Target,
        now: DateTime<Utc>,
    ) -> Result<Grant, Refusal> {
        // 1. No actor -> signIn
        let actor = actor.ok_or(Refusal::SignIn)?;

        // 2. Organisation -> outOfScope
        if actor.organization_id != target.organization_id {
            return Err(Refusal::OutOfScope);
        }

        // 3. Permission and branch
        let qualifying: Vec<_> = qualifying(actor, permission, now).collect();

        if qualifying.is_empty() {
            return Err(Refusal::RoleNotAllowed);
        }

        let grant_branch = match &target.branch {
            Some(target_b) => {
                let in_scope = qualifying
                    .iter()
                    .any(|r| r.branch.is_none() || r.branch.as_ref() == Some(target_b));
                if in_scope {
                    target_b.clone()
                } else {
                    return Err(Refusal::OutOfScope);
                }
            }
            None => {
                // SB-1 §14.3: the session branch, when a qualifying row covers it; otherwise the arms below.
                let session = actor.branch.as_ref().filter(|b| {
                    qualifying
                        .iter()
                        .any(|r| r.branch.is_none() || r.branch.as_ref() == Some(*b))
                });
                let has_org_wide = qualifying.iter().any(|r| r.branch.is_none());
                if let Some(b) = session {
                    b.clone()
                } else if has_org_wide {
                    BranchId::new("MAIN")
                } else {
                    let distinct_branches: HashSet<_> = qualifying
                        .iter()
                        .filter_map(|r| r.branch.as_ref())
                        .collect();
                    if distinct_branches.len() == 1 {
                        (*distinct_branches.into_iter().next().unwrap()).clone()
                    } else {
                        return Err(Refusal::OutOfScope);
                    }
                }
            }
        };

        // 4. Owner check for CustomerSelf
        if permission == Permission::CustomerSelf {
            match (&actor.customer_id, &target.customer_id) {
                (Some(actor_cust), Some(target_cust)) if actor_cust == target_cust => {}
                _ => return Err(Refusal::OutOfScope),
            }
        }

        Ok(Grant {
            branch: grant_branch,
            roles_used: qualifying.into_iter().map(|r| r.role).collect(),
        })
    }
}
