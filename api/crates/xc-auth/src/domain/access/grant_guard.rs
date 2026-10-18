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
        let _ = (actor, target, action, owners_left);
        todo!("RBAC function ticket: GrantGuard::check (§3.1)")
    }
}
