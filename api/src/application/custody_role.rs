//! The one mapping from a person's role to its part in custody.
//!
//! Three callers need it — the RPC actor, the saver of an exchange, an
//! employee row — and it lives in the root package because it needs both
//! `Role` (`xc-auth`) and `CustodyRole` (`xc-shift`), and a context crate may
//! not depend on another (`docs/design/xchange-v3.md` §5.2).

use xc_auth::domain::policy::value_objects::role::Role;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;

/// The part a role plays in custody. The four roles with none (Rates, System,
/// Documents, Document review) are `Other`: they clock in and out and nothing
/// else.
pub fn custody_role_of(role: Role) -> CustodyRole {
    match role {
        Role::Owner => CustodyRole::Owner,
        Role::Front => CustodyRole::Front,
        Role::Finance => CustodyRole::Finance,
        Role::Monitor => CustodyRole::Monitor,
        Role::Manager => CustodyRole::Manager,
        Role::Backend => CustodyRole::Backend,
        Role::Rates | Role::System | Role::Documents | Role::DocumentReview => CustodyRole::Other,
    }
}

/// The custody role of an employee's role as stored. A role with no part in
/// custody, and anything unknown, is `Other`; an employee is never `Owner`.
pub fn employee_custody_role(stored: &str) -> CustodyRole {
    match Role::parse(stored) {
        None | Some(Role::Owner) => CustodyRole::Other,
        Some(role) => custody_role_of(role),
    }
}
