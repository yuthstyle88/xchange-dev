//! What a v2 surface knows about a caller, read once from an `access::Actor` (OR-7, #1026).
//!
//! The compat routes, the save steps, the kiosk and custody mappings each need one answer:
//! is this the owner, or staff with which role at which branch. Before OR-7 that answer was an old
//! `policy::value_objects::Actor`, guessed twice (`legacy_actor` in the extractor and
//! `SaveActor::from_actor_at`). `Standing::of` is the one place the rule lives now; every reader
//! takes it from here, and the old types go with #1027.
//!
//! The rule is unchanged (each outcome stays what the old path gave the mapped role, OR-7):
//! - org-wide EXECUTIVE and org-wide ADMIN, both valid at `now` → the owner;
//! - else the first valid role in the order TELLER, BRANCH_MANAGER, RATE_MANAGER, FINANCE, ADMIN,
//!   VERIFICATION_REVIEWER → staff with that role, at that role's branch, else `MAIN`;
//! - else (EXECUTIVE alone, MARKETING, CUSTOMER, nothing valid) → no standing.
//!
//! The audit text of a standing keeps v3's spelling ([`StandingKind::audit_text`]).

use chrono::{DateTime, Utc};
use xc_kernel::ids::BranchId;

use super::actor::Actor;
use super::role::Role;
use super::scoped_role::ScopedRole;

/// The branch a staff standing falls back to when its role is organisation-wide.
pub const FALLBACK_BRANCH: &str = "MAIN";

/// The staff roles a standing can carry, highest priority first.
pub const STAFF_PRIORITY: [Role; 6] = [
    Role::Teller,
    Role::BranchManager,
    Role::RateManager,
    Role::Finance,
    Role::Admin,
    Role::VerificationReviewer,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandingKind {
    Owner,
    /// Staff, with the role that decided the standing (one of [`STAFF_PRIORITY`]).
    Staff(Role),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    pub user_id: String,
    pub kind: StandingKind,
    /// The staff role's branch (or [`FALLBACK_BRANCH`]); `None` for the owner.
    pub branch: Option<BranchId>,
}

fn valid_at(role: &ScopedRole, now: DateTime<Utc>) -> bool {
    role.valid_from <= now && role.valid_to.is_none_or(|to| to > now)
}

impl Standing {
    /// The caller's standing at `now`, or `None` when no v2 surface would know them.
    pub fn of(actor: &Actor, now: DateTime<Utc>) -> Option<Standing> {
        let valid_roles: Vec<&ScopedRole> =
            actor.roles.iter().filter(|r| valid_at(r, now)).collect();

        let has_org_exec = valid_roles
            .iter()
            .any(|r| r.role == Role::Executive && r.branch.is_none());
        let has_org_admin = valid_roles
            .iter()
            .any(|r| r.role == Role::Admin && r.branch.is_none());

        if has_org_exec && has_org_admin {
            return Some(Standing {
                user_id: actor.user_id.clone(),
                kind: StandingKind::Owner,
                branch: None,
            });
        }

        for role in STAFF_PRIORITY {
            if let Some(r) = valid_roles.iter().find(|r| r.role == role) {
                let branch = r
                    .branch
                    .clone()
                    .unwrap_or_else(|| BranchId::new(FALLBACK_BRANCH));
                return Some(Standing {
                    user_id: actor.user_id.clone(),
                    kind: StandingKind::Staff(role),
                    branch: Some(branch),
                });
            }
        }

        None
    }

    pub fn is_owner(&self) -> bool {
        self.kind == StandingKind::Owner
    }

    /// The staff role; `None` for the owner.
    pub fn staff_role(&self) -> Option<Role> {
        match self.kind {
            StandingKind::Owner => None,
            StandingKind::Staff(role) => Some(role),
        }
    }
}

impl StandingKind {
    /// The audit `actor_role` text, spelled as v3 stored it (unchanged by OR-7: the stored text stays what
    /// it was, `or7_5_save_actor_test.rs:458`, ruling 5 on #1020): OWNER, FRONT (TELLER), MANAGER
    /// (BRANCH_MANAGER), RATES (RATE_MANAGER), FINANCE, SYSTEM (ADMIN), DOCUMENT_REVIEW
    /// (VERIFICATION_REVIEWER). A role outside [`STAFF_PRIORITY`] never reaches a standing.
    pub fn audit_text(self) -> &'static str {
        match self {
            StandingKind::Owner => "OWNER",
            StandingKind::Staff(Role::Teller) => "FRONT",
            StandingKind::Staff(Role::BranchManager) => "MANAGER",
            StandingKind::Staff(Role::RateManager) => "RATES",
            StandingKind::Staff(Role::Finance) => "FINANCE",
            StandingKind::Staff(Role::Admin) => "SYSTEM",
            StandingKind::Staff(Role::VerificationReviewer) => "DOCUMENT_REVIEW",
            StandingKind::Staff(other) => other.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-18T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn actor(roles: &[(Role, Option<&str>)]) -> Actor {
        let since = at() - Duration::days(1);
        Actor::new(
            "u1",
            "org",
            roles
                .iter()
                .map(|(r, b)| ScopedRole::new(*r, b.map(BranchId::new), since, None))
                .collect(),
            None,
        )
    }

    #[test]
    fn org_wide_executive_and_admin_is_the_owner() {
        let s = Standing::of(
            &actor(&[(Role::Executive, None), (Role::Admin, None)]),
            at(),
        )
        .unwrap();
        assert!(s.is_owner());
        assert_eq!(s.branch, None);
    }

    #[test]
    fn executive_alone_has_no_standing() {
        assert_eq!(Standing::of(&actor(&[(Role::Executive, None)]), at()), None);
    }

    #[test]
    fn branch_scoped_admin_with_executive_is_staff_not_owner() {
        let s = Standing::of(
            &actor(&[(Role::Executive, None), (Role::Admin, Some("B1"))]),
            at(),
        )
        .unwrap();
        assert_eq!(s.kind, StandingKind::Staff(Role::Admin));
        assert_eq!(s.branch, Some(BranchId::new("B1")));
    }

    #[test]
    fn teller_wins_over_branch_manager() {
        let s = Standing::of(
            &actor(&[
                (Role::BranchManager, Some("B2")),
                (Role::Teller, Some("B1")),
            ]),
            at(),
        )
        .unwrap();
        assert_eq!(s.kind, StandingKind::Staff(Role::Teller));
        assert_eq!(s.branch, Some(BranchId::new("B1")));
    }

    #[test]
    fn org_wide_staff_role_falls_back_to_main() {
        let s = Standing::of(&actor(&[(Role::Finance, None)]), at()).unwrap();
        assert_eq!(s.branch, Some(BranchId::new(FALLBACK_BRANCH)));
    }

    #[test]
    fn customer_alone_has_no_standing() {
        assert_eq!(Standing::of(&actor(&[(Role::Customer, None)]), at()), None);
    }

    // #3929 ruling (a): MARKETING is an employee role, so it gets a staff standing (lowest priority)
    #[test]
    fn marketing_alone_gets_a_staff_standing() {
        let at_main = Standing::of(&actor(&[(Role::Marketing, Some("MAIN"))]), at())
            .expect("MARKETING at MAIN must have a standing");
        assert_eq!(at_main.kind, StandingKind::Staff(Role::Marketing));
        assert_eq!(at_main.branch, Some(BranchId::new("MAIN")));

        let org_wide = Standing::of(&actor(&[(Role::Marketing, None)]), at())
            .expect("org-wide MARKETING must have a standing");
        assert_eq!(org_wide.kind, StandingKind::Staff(Role::Marketing));
        assert_eq!(org_wide.branch, Some(BranchId::new(FALLBACK_BRANCH)));
    }

    #[test]
    fn marketing_is_outranked_by_any_other_staff_role() {
        for other in [
            Role::Teller,
            Role::BranchManager,
            Role::RateManager,
            Role::Finance,
            Role::Admin,
            Role::VerificationReviewer,
        ] {
            let s = Standing::of(
                &actor(&[(Role::Marketing, Some("MAIN")), (other, Some("B1"))]),
                at(),
            )
            .unwrap_or_else(|| panic!("MARKETING + {other:?} must have a standing"));
            assert_eq!(s.kind, StandingKind::Staff(other), "MARKETING + {other:?}");
            assert_eq!(s.branch, Some(BranchId::new("B1")), "MARKETING + {other:?}");
        }
    }

    #[test]
    fn expired_roles_do_not_count() {
        let mut a = actor(&[(Role::Teller, Some("B1"))]);
        a.roles[0].valid_to = Some(at());
        assert_eq!(Standing::of(&a, at()), None);
    }

    #[test]
    fn audit_text_keeps_the_v3_spelling() {
        for (kind, text) in [
            (StandingKind::Owner, "OWNER"),
            (StandingKind::Staff(Role::Teller), "FRONT"),
            (StandingKind::Staff(Role::BranchManager), "MANAGER"),
            (StandingKind::Staff(Role::RateManager), "RATES"),
            (StandingKind::Staff(Role::Finance), "FINANCE"),
            (StandingKind::Staff(Role::Admin), "SYSTEM"),
            (
                StandingKind::Staff(Role::VerificationReviewer),
                "DOCUMENT_REVIEW",
            ),
        ] {
            assert_eq!(kind.audit_text(), text, "{kind:?}");
        }
    }
}
