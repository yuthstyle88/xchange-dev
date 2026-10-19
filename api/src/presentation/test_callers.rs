//! Test builders of [`Callers`] without the old v2 identity types (OR-7.m #1027).
//!
//! Each builds exactly what `old_cookie_callers` built from a v2 cookie actor: the seeded
//! organisation, roles valid from the Unix epoch, and the owner first. Integration tests move
//! `Sessions { owner, staff }` onto these one file at a time; then password login is deleted.
//!
//! A v2 role becomes these Blueprint roles (design `org-roles-branches.md` §3.3):
//!
//! | v2 role | `staff(.., roles, ..)` |
//! |---|---|
//! | FRONT | `&[Role::Teller]` |
//! | BACKEND, MANAGER | `&[Role::BranchManager]` |
//! | RATES | `&[Role::RateManager]` |
//! | FINANCE | `&[Role::Finance]` |
//! | SYSTEM | `&[Role::Admin]` |
//! | MONITOR, DOCUMENTS, DOCUMENT_REVIEW | `&[Role::VerificationReviewer]` |
//!
//! `Actor::owner(id)` becomes [`owner`]`(id)`.

use chrono::{DateTime, Utc};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;

use super::pages::{Callers, SEEDED_XC_ORG_ID};

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// The owner `id`: organisation-wide EXECUTIVE and ADMIN.
pub fn owner(id: &str) -> Actor {
    staff_or_owner(id, &[Role::Executive, Role::Admin], None)
}

/// A staff member `id` holding each of `roles` at `branch`.
pub fn staff(id: &str, roles: &[Role], branch: BranchId) -> Actor {
    staff_or_owner(id, roles, Some(branch))
}

fn staff_or_owner(id: &str, roles: &[Role], branch: Option<BranchId>) -> Actor {
    let roles = roles
        .iter()
        .map(|&role| ScopedRole::new(role, branch.clone(), epoch(), None))
        .collect();
    Actor::new(id, SEEDED_XC_ORG_ID, roles, None)
}

/// The callers a request with these sessions carries: `user` is the owner, else the staff member;
/// `staff_user` is the staff member.
pub fn callers(owner: Option<Actor>, staff: Option<Actor>) -> Callers {
    Callers {
        user: owner.or_else(|| staff.clone()),
        staff_user: staff,
    }
}

/// No session at all.
pub fn nobody() -> Callers {
    callers(None, None)
}

#[cfg(test)]
mod tests {
    //! Bridge test: deleted with `old_cookie_callers` (OR-7.m's last ticket).
    use xc_auth::domain::policy::value_objects::actor::Actor as V3Actor;
    use xc_auth::domain::policy::value_objects::role::Role as V3Role;

    use super::*;
    use crate::presentation::pages::{Sessions, old_cookie_callers};

    fn same(left: &Callers, right: &Callers) {
        assert_eq!(left.user, right.user);
        assert_eq!(left.staff_user, right.staff_user);
    }

    #[test]
    fn every_v2_actor_builds_the_same_callers_as_the_old_cookies() {
        let branch = BranchId::new("B1");
        let table: [(V3Role, &[Role]); 9] = [
            (V3Role::Front, &[Role::Teller]),
            (V3Role::Backend, &[Role::BranchManager]),
            (V3Role::Manager, &[Role::BranchManager]),
            (V3Role::Rates, &[Role::RateManager]),
            (V3Role::Finance, &[Role::Finance]),
            (V3Role::System, &[Role::Admin]),
            (V3Role::Monitor, &[Role::VerificationReviewer]),
            (V3Role::Documents, &[Role::VerificationReviewer]),
            (V3Role::DocumentReview, &[Role::VerificationReviewer]),
        ];
        for (v3, roles) in table {
            let old = V3Actor::staff("e1", v3, branch.clone()).expect("a staff role");
            for with_owner in [false, true] {
                let expected = old_cookie_callers(
                    Sessions {
                        owner: with_owner.then(|| V3Actor::owner("owner")),
                        staff: Some(old.clone()),
                    },
                    SEEDED_XC_ORG_ID,
                );
                let built = callers(
                    with_owner.then(|| owner("owner")),
                    Some(staff("e1", roles, branch.clone())),
                );
                same(&built, &expected);
            }
        }
        let owner_only = old_cookie_callers(
            Sessions {
                owner: Some(V3Actor::owner("owner")),
                staff: None,
            },
            SEEDED_XC_ORG_ID,
        );
        same(&callers(Some(owner("owner")), None), &owner_only);
        let none = old_cookie_callers(
            Sessions {
                owner: None,
                staff: None,
            },
            SEEDED_XC_ORG_ID,
        );
        same(&nobody(), &none);
    }
}
