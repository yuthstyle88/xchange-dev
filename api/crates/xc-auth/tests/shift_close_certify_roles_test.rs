//! Shift-close certification — up-front tests: manager level or above certifies a close count (owner, 2026-10-25).
//!
//! Written before the implementation by the ticket author (claude/bc8fd29a, xchange BE lane controller).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Owner, verbatim (claude/8be3a451's session, recorded on #564): *"ต้องให้ เจ้าหน้าที่ ระดับ ผจก ขึ้นไป รับรอง แล้ว มีรายงานแจ้ง"*
//! and *"Yes: Executive and Admin can also certify a shift close, keeping verifier ≠ counter and the finance gate, with
//! a notice on every certification and on waiting counts."* In claude/bc8fd29a's session the owner accepted that Admin
//! then also reads the staff day report (#564 issuecomment-6392731530).
//!
//! The defect at origin/main: only `BranchManager` holds `ShiftCloseCertify` (`role_grants.rs:237`), and a branch
//! manager covers one branch (`scope_rule.rs:33-38`). So a branch whose manager is away, or whose only manager did the
//! count, cannot close a shift.
//!
//! What this file pins ([required]):
//! - C1: `ShiftCloseCertify` is held by BranchManager, Executive and Admin, and by no other role.
//! - C2: an Executive (organisation-wide) may `countVerify` at any branch.
//! - C3: an organisation-wide Admin may `countVerify` at any branch; an Admin scoped to B1 may at B1 and not at B2.
//! - C4: a BranchManager of B1 may at B1 and not at B2 (unchanged).
//! - C5: a Teller may not.
//!
//! No database: `Policy::authorize` reads `RoleGrantsSnapshot::seeded()`.

mod policy_common;

use policy_common::*;

fn verify_at(who: &Actor, at: &str) -> Result<Grant, PolicyError> {
    Policy::authorize(
        Some(who),
        EntryPath::Rpc,
        &action("countVerify"),
        Some(&branch(at)),
        now(),
    )
}

#[test]
fn c1_shift_close_certify_is_held_by_branch_manager_executive_and_admin_only() {
    for r in BLUEPRINT {
        let holds = RoleGrants::seed(r).contains(&Permission::ShiftCloseCertify);
        let expected = matches!(r, Role::BranchManager | Role::Executive | Role::Admin);
        assert_eq!(holds, expected, "{r:?} holds ShiftCloseCertify: {holds}");
    }
}

#[test]
fn c2_an_executive_may_certify_at_any_branch() {
    let exec = staff(Role::Executive, "B1");
    assert!(verify_at(&exec, "B1").is_ok());
    assert!(verify_at(&exec, "B2").is_ok());
}

#[test]
fn c3_an_admin_certifies_where_their_scope_covers() {
    let org_admin = user("U-ADMIN-ORG", vec![row(Role::Admin, None)]);
    assert!(verify_at(&org_admin, "B1").is_ok());
    assert!(verify_at(&org_admin, "B2").is_ok());

    let b1_admin = user("U-ADMIN-B1", vec![row(Role::Admin, Some("B1"))]);
    assert!(verify_at(&b1_admin, "B1").is_ok());
    assert!(
        verify_at(&b1_admin, "B2").is_err(),
        "an Admin scoped to B1 does not certify at B2"
    );
}

#[test]
fn c4_a_branch_manager_certifies_only_at_their_branch() {
    let manager = staff(Role::BranchManager, "B1");
    assert!(verify_at(&manager, "B1").is_ok());
    assert!(verify_at(&manager, "B2").is_err());
}

#[test]
fn c5_a_teller_may_not_certify() {
    let teller = staff(Role::Teller, "B1");
    assert!(verify_at(&teller, "B1").is_err());
}
