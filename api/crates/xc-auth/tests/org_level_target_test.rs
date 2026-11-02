//! #5514 (design `docs/design/org-wide-actor-branch.md` §9.4, rows O-1..O-6): an organisation-level target needs an
//! organisation-wide row; an unnamed target keeps SB-1 §14.3. Written before the implementation by the ticket author
//! (claude/2ca5874b). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Pure: no database. The time is passed in, never read from the clock.
//!
//! Signatures these tests assume (§9.2 [required]):
//! - `Target::organisation(organization_id: &str) -> Target` (an object of the whole organisation);
//! - `Target::at_branch(organization_id: &str, branch: BranchId) -> Target`, equal to `Target::new(org, Some(b), None)`;
//! - `Target::new(org, None, None)` stays the unnamed target.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::authorizer::{Authorizer, Refusal, Target};
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 29, 12, 0, 0).unwrap()
}

fn admin(branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(
        Role::Admin,
        branch.map(BranchId::new),
        now() - Duration::days(1),
        None,
    )
}

fn actor(roles: Vec<ScopedRole>, session: Option<&str>) -> Actor {
    Actor::new("user-1", ORG, roles, None).with_branch(session.map(BranchId::new))
}

fn check(a: &Actor, target: &Target) -> Result<BranchId, Refusal> {
    // Restated for #6004 (user-management-rbac.md §11.11): SystemSettings is EXECUTIVE only; use BranchesManage to test ADMIN org-level and branch-scoped grants.
    Authorizer::check(Some(a), Permission::BranchesManage, target, now()).map(|g| g.branch)
}

#[test]
fn o1_an_organisation_wide_row_in_a_session_acts_on_the_organisation_at_the_session_branch() {
    let a = actor(vec![admin(None)], Some("B1"));
    assert_eq!(
        check(&a, &Target::organisation(ORG)),
        Ok(BranchId::new("B1"))
    );
}

#[test]
fn o2_a_branch_scoped_row_is_refused_an_organisation_target_even_at_its_own_session_branch() {
    let a = actor(vec![admin(Some("B1"))], Some("B1"));
    assert_eq!(
        check(&a, &Target::organisation(ORG)),
        Err(Refusal::OutOfScope)
    );
}

#[test]
fn o3_a_branch_row_beside_an_organisation_wide_row_still_acts_on_the_organisation() {
    let a = actor(vec![admin(Some("B1")), admin(None)], Some("B1"));
    assert_eq!(
        check(&a, &Target::organisation(ORG)),
        Ok(BranchId::new("B1"))
    );
}

#[test]
fn o4_an_organisation_wide_row_without_a_session_branch_is_refused_and_no_branch_is_invented() {
    let a = actor(vec![admin(None)], None);
    assert_eq!(
        check(&a, &Target::organisation(ORG)),
        Err(Refusal::OutOfScope)
    );
}

#[test]
fn o5_an_unnamed_target_still_grants_a_branch_row_its_session_branch() {
    let a = actor(vec![admin(Some("B1"))], Some("B1"));
    assert_eq!(
        check(&a, &Target::new(ORG, None, None)),
        Ok(BranchId::new("B1"))
    );
}

#[test]
fn o6_an_unnamed_target_still_grants_a_single_branch_row_without_a_session() {
    let a = actor(vec![admin(Some("B1"))], None);
    assert_eq!(
        check(&a, &Target::new(ORG, None, None)),
        Ok(BranchId::new("B1"))
    );
}

#[test]
fn another_organisation_is_refused_an_organisation_target() {
    let a = Actor::new(
        "user-1",
        "00000000-0000-0000-0000-000000000002",
        vec![admin(None)],
        None,
    )
    .with_branch(Some(BranchId::new("B1")));
    assert_eq!(
        check(&a, &Target::organisation(ORG)),
        Err(Refusal::OutOfScope)
    );
}

#[test]
fn at_branch_is_the_named_branch_target() {
    assert_eq!(
        Target::at_branch(ORG, BranchId::new("B2")),
        Target::new(ORG, Some(BranchId::new("B2")), None)
    );
}
