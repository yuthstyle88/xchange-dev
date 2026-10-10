use chrono::{DateTime, Utc};

use super::actor::Actor;
use super::role::Role;
use super::scoped_role::ScopedRole;
use crate::domain::policy::value_objects::actor::Actor as V3Actor;
use crate::domain::policy::value_objects::actor_kind::ActorKind;
use crate::domain::policy::value_objects::role::Role as V3Role;

/// The Blueprint roles a v3 role becomes (§3.3).
pub fn blueprint_roles(v3: V3Role) -> &'static [Role] {
    match v3 {
        V3Role::Owner => &[Role::Executive, Role::Admin],
        V3Role::Front => &[Role::Teller],
        V3Role::Backend | V3Role::Manager => &[Role::BranchManager],
        V3Role::Rates => &[Role::RateManager],
        V3Role::Finance => &[Role::Finance],
        V3Role::System => &[Role::Admin],
        V3Role::Monitor | V3Role::Documents | V3Role::DocumentReview => {
            &[Role::VerificationReviewer]
        }
    }
}

/// The user B1 makes of a v3 actor: the owner's roles organisation-wide, an employee's on their
/// branch, each valid from `since`.
pub fn actor_from_v3(v3: &V3Actor, organization_id: &str, since: DateTime<Utc>) -> Actor {
    let branch = if v3.kind() == ActorKind::Owner {
        None
    } else {
        v3.branch_id().cloned()
    };
    let roles = blueprint_roles(v3.role())
        .iter()
        .map(|&role| ScopedRole::new(role, branch.clone(), since, None))
        .collect();
    Actor::new(v3.id(), organization_id, roles, None)
}
