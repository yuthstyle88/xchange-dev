//! RBAC C1 `GrantGuard`: who may change whom (design `user-management-rbac.md` §3.1, owner Q3/Q4 = A).
//! Skeleton on `feat/rbac`: the body is a function ticket.

use super::scope_rule::ScopeError;
use super::target_user::TargetUser;
use super::{Actor, ScopedRole};

/// One change an actor asks to make to a user (§3.1). Grant/Revoke carry the branch (ruling #564 issuecomment-6246076019 Q2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountAction {
    Grant(ScopedRole),
    Revoke(ScopedRole),
    Disable,
    Suspend,
    Resume,
    Enable,
    ResetPassword,
    EditProfile,
}

/// Why `GrantGuard::check` refuses (§3.1 rules 1–7).
#[derive(Debug, PartialEq, Eq)]
pub enum GrantRefusal {
    NoScope,
    SelfChange,
    ProtectedTarget,
    ProtectedRole,
    EnableNeedsExecutive,
    LastOwner,
    Scope(ScopeError),
}

pub struct GrantGuard;

impl GrantGuard {
    /// `owners_left`: ACTIVE users of the target's organisation holding live organisation-wide EXECUTIVE or ADMIN, the target included.
    pub fn check(
        actor: &Actor,
        target: &TargetUser,
        action: &AccountAction,
        owners_left: usize,
    ) -> Result<(), GrantRefusal> {
        use super::Role;
        use super::scope_rule::ScopeRule;
        use crate::domain::user::UserStatus;
        use chrono::Utc;

        let now = Utc::now();
        let is_live = |r: &ScopedRole| r.valid_from <= now && r.valid_to.is_none_or(|to| to > now);
        let actor_live: Vec<&ScopedRole> = actor.roles.iter().filter(|r| is_live(r)).collect();
        let target_live: Vec<ScopedRole> = target
            .roles
            .iter()
            .filter(|r| is_live(r))
            .cloned()
            .collect();

        let admin_scopes: Vec<Option<&xc_kernel::ids::BranchId>> = actor_live
            .iter()
            .filter(|r| r.role == Role::Admin)
            .map(|r| r.branch.as_ref())
            .collect();
        if admin_scopes.is_empty() {
            return Err(GrantRefusal::NoScope);
        }
        let actor_has_org_admin = admin_scopes.contains(&None);
        let actor_has_exec = actor_live.iter().any(|r| r.role == Role::Executive);
        let actor_is_owner = actor_has_org_admin && actor_has_exec;

        let covers = |branch: Option<&xc_kernel::ids::BranchId>| {
            actor_has_org_admin || (branch.is_some() && admin_scopes.contains(&branch))
        };
        let in_scope = match action {
            AccountAction::Grant(r) | AccountAction::Revoke(r) => covers(r.branch.as_ref()),
            _ => target_live.iter().all(|r| covers(r.branch.as_ref())),
        };
        if !in_scope {
            return Err(GrantRefusal::NoScope);
        }

        if actor.user_id == target.id {
            return if matches!(action, AccountAction::EditProfile) {
                Ok(())
            } else {
                Err(GrantRefusal::SelfChange)
            };
        }

        if target_live
            .iter()
            .any(|r| matches!(r.role, Role::Admin | Role::Executive))
            && !actor_is_owner
        {
            return Err(GrantRefusal::ProtectedTarget);
        }

        if matches!(action, AccountAction::Grant(r) if matches!(r.role, Role::Admin | Role::Executive))
            && !actor_is_owner
        {
            return Err(GrantRefusal::ProtectedRole);
        }

        if matches!(action, AccountAction::Enable) && !actor_has_exec {
            return Err(GrantRefusal::EnableNeedsExecutive);
        }

        let target_is_active_owner = target.status == UserStatus::Active
            && target_live
                .iter()
                .any(|r| r.role == Role::Executive && r.branch.is_none())
            && target_live
                .iter()
                .any(|r| r.role == Role::Admin && r.branch.is_none());
        if target_is_active_owner && owners_left <= 1 {
            let loses_owner = match action {
                AccountAction::Suspend | AccountAction::Disable => true,
                AccountAction::Revoke(r)
                    if r.branch.is_none() && matches!(r.role, Role::Executive | Role::Admin) =>
                {
                    target_live
                        .iter()
                        .filter(|t| t.role == r.role && t.branch.is_none())
                        .count()
                        <= 1
                }
                _ => false,
            };
            if loses_owner {
                return Err(GrantRefusal::LastOwner);
            }
        }

        if let AccountAction::Grant(r) = action {
            ScopeRule::check(r.role, r.branch.as_ref(), &target_live)
                .map_err(GrantRefusal::Scope)?;
        }

        Ok(())
    }
}
