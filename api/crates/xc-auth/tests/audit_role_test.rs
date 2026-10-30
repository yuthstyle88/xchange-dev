//! #5630 — `access::audit_role`: one rule for an audit row's `actor_role` (design
//! `docs/design/org-wide-actor-branch.md` §14.4-§14.5; rulings Q-1 A and Q-2 A on #5661).
//! Written before the implementation by the sub-controller (claude/studio-claude-17). Do not edit,
//! delete or weaken; add tests in a file of your own. Green on the skeleton: the skeleton is what
//! it tests.

use chrono::{DateTime, Duration, Utc};
use xc_auth::domain::access::{Actor, Role, ScopedRole, audit_role};
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

/// A-1 (Q-2 A): the granting role wins, whatever else the caller holds.
#[test]
fn a1_the_granting_role_is_written() {
    let a = actor(&[(Role::Teller, Some("B1")), (Role::Finance, Some("B1"))]);
    assert_eq!(audit_role(&[Role::Finance], &a, at()), "FINANCE");
    assert_eq!(audit_role(&[Role::Teller], &a, at()), "TELLER");
}

/// A-2: the first granting role, when several rows qualified.
#[test]
fn a2_the_first_of_several_granting_roles() {
    let a = actor(&[(Role::Executive, None), (Role::Admin, None)]);
    assert_eq!(
        audit_role(&[Role::Admin, Role::Executive], &a, at()),
        "ADMIN"
    );
}

/// A-3 (Q-2 B): no grant ⇒ the `Standing` order, in Blueprint names (Q-1 A).
#[test]
fn a3_without_a_grant_the_standing_order_in_blueprint_names() {
    for (roles, text) in [
        (&[(Role::Teller, Some("B1"))][..], "TELLER"),
        (&[(Role::BranchManager, Some("B1"))][..], "BRANCH_MANAGER"),
        (&[(Role::RateManager, Some("B1"))][..], "RATE_MANAGER"),
        (&[(Role::Finance, Some("B1"))][..], "FINANCE"),
        (&[(Role::Admin, None)][..], "ADMIN"),
        (
            &[(Role::VerificationReviewer, Some("B1"))][..],
            "VERIFICATION_REVIEWER",
        ),
        (&[(Role::Marketing, None)][..], "MARKETING"),
        (
            &[(Role::Finance, Some("B1")), (Role::Teller, Some("B1"))][..],
            "TELLER",
        ),
    ] {
        assert_eq!(audit_role(&[], &actor(roles), at()), text, "{roles:?}");
    }
}

/// A-4: the owner and an executive alone are both written `EXECUTIVE` (Q-2 B ruling).
#[test]
fn a4_the_owner_and_an_executive_are_executive() {
    assert_eq!(
        audit_role(
            &[],
            &actor(&[(Role::Executive, None), (Role::Admin, None)]),
            at()
        ),
        "EXECUTIVE"
    );
    assert_eq!(
        audit_role(&[], &actor(&[(Role::Executive, None)]), at()),
        "EXECUTIVE"
    );
}

/// A-5: no grant and no standing ⇒ `STAFF`. Never `TELLER`, `""`, `UNKNOWN` or a join.
#[test]
fn a5_otherwise_staff() {
    assert_eq!(audit_role(&[], &actor(&[]), at()), "STAFF");
    assert_eq!(
        audit_role(&[], &actor(&[(Role::Customer, None)]), at()),
        "STAFF"
    );
    let mut expired = actor(&[(Role::Teller, Some("B1"))]);
    expired.roles[0].valid_to = Some(at());
    assert_eq!(audit_role(&[], &expired, at()), "STAFF");
}

/// A-6: no V.9 name is ever written.
#[test]
fn a6_no_v9_name() {
    let v9 = [
        "OWNER",
        "FRONT",
        "MANAGER",
        "RATES",
        "SYSTEM",
        "DOCUMENT_REVIEW",
    ];
    for roles in [
        &[(Role::Executive, None), (Role::Admin, None)][..],
        &[(Role::Teller, Some("B1"))][..],
        &[(Role::BranchManager, Some("B1"))][..],
        &[(Role::RateManager, Some("B1"))][..],
        &[(Role::Admin, None)][..],
        &[(Role::VerificationReviewer, Some("B1"))][..],
    ] {
        let text = audit_role(&[], &actor(roles), at());
        assert!(!v9.contains(&text), "{roles:?} wrote {text}");
    }
}
