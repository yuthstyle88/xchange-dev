//! Who may enter `/staff` (design `docs/design/blueprint/user-management-rbac.md` §11.8).

use chrono::{DateTime, Utc};

use super::actor::Actor;
use super::role::Role;

/// True iff some `ScopedRole` on `actor` is valid at `now` (`valid_from <= now` and `valid_to` is
/// `None` or `> now`) and its base `role != Role::Customer`.
pub fn is_staff_caller(actor: &Actor, now: DateTime<Utc>) -> bool {
    actor.roles.iter().any(|r| {
        r.role != Role::Customer && r.valid_from <= now && r.valid_to.is_none_or(|to| to > now)
    })
}
