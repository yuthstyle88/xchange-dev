//! #5618 — `Standing::acting`: what a v3 command acts with after its permission check
//! (design `docs/design/org-wide-actor-branch.md` §12, rows S-1..S-7).
//! Written before the implementation by the sub-controller (claude/studio-claude-17). Do not edit,
//! delete or weaken; add tests in a file of your own. Green on the skeleton: the skeleton is what
//! it tests.

use chrono::{DateTime, Duration, Utc};
use xc_auth::domain::access::{Actor, Role, ScopedRole, Standing, StandingKind};
use xc_kernel::ids::BranchId;

fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-30T00:00:00Z")
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

/// S-1: EXECUTIVE alone, organisation-wide, acts as `Executive` with no branch; `Standing::of`
/// still answers `None` (v2 surfaces unchanged).
#[test]
fn s1_an_executive_alone_acts_as_executive() {
    let a = actor(&[(Role::Executive, None)]);
    assert_eq!(Standing::of(&a, at()), None);
    let s = Standing::acting(&a, at()).expect("an org-wide EXECUTIVE acts");
    assert_eq!(s.kind, StandingKind::Executive);
    assert_eq!(s.branch, None);
    assert_eq!(s.user_id, "u1");
    assert!(!s.is_owner(), "B3: OWNER is EXECUTIVE + ADMIN");
    assert_eq!(s.staff_role(), None);
}

/// S-2: every caller `Standing::of` answers acts with that same standing.
#[test]
fn s2_acting_is_standing_of_whenever_of_answers() {
    for roles in [
        &[(Role::Executive, None), (Role::Admin, None)][..],
        &[(Role::Executive, None), (Role::Teller, Some("B1"))][..],
        &[(Role::Executive, None), (Role::Admin, Some("B1"))][..],
        &[(Role::RateManager, Some("B2"))][..],
        &[(Role::Finance, None)][..],
    ] {
        let a = actor(roles);
        assert_eq!(
            Standing::acting(&a, at()),
            Standing::of(&a, at()),
            "{roles:?}"
        );
        assert!(Standing::of(&a, at()).is_some(), "{roles:?}");
    }
}

/// S-3: a branch-scoped EXECUTIVE alone gets no standing.
#[test]
fn s3_a_branch_scoped_executive_does_not_act() {
    assert_eq!(
        Standing::acting(&actor(&[(Role::Executive, Some("B1"))]), at()),
        None
    );
}

/// S-4: an EXECUTIVE row that is not valid at `now` does not count.
#[test]
fn s4_an_expired_executive_does_not_act() {
    let mut a = actor(&[(Role::Executive, None)]);
    a.roles[0].valid_to = Some(at());
    assert_eq!(Standing::acting(&a, at()), None);
    let mut b = actor(&[(Role::Executive, None)]);
    b.roles[0].valid_from = at() + Duration::seconds(1);
    assert_eq!(Standing::acting(&b, at()), None);
}

/// S-5: a customer, or nobody, gets no standing.
#[test]
fn s5_a_customer_does_not_act() {
    assert_eq!(
        Standing::acting(&actor(&[(Role::Customer, None)]), at()),
        None
    );
    assert_eq!(Standing::acting(&actor(&[]), at()), None);
}

/// S-6: the audit text: `EXECUTIVE` for an executive alone, the standing's text otherwise, and
/// `STAFF` when there is no standing (never a refusal).
#[test]
fn s6_audit_text_of_every_caller() {
    for (roles, text) in [
        (&[(Role::Executive, None)][..], "EXECUTIVE"),
        (&[(Role::Executive, None), (Role::Admin, None)][..], "OWNER"),
        (&[(Role::Admin, None)][..], "SYSTEM"),
        (&[(Role::Teller, Some("B1"))][..], "FRONT"),
        (&[(Role::Customer, None)][..], "STAFF"),
    ] {
        assert_eq!(
            Standing::audit_text_of(&actor(roles), at()),
            text,
            "{roles:?}"
        );
    }
    assert_eq!(StandingKind::Executive.audit_text(), "EXECUTIVE");
}

/// S-7: the owner's audit text and every staff text are unchanged by the new kind.
#[test]
fn s7_existing_audit_texts_are_unchanged() {
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
