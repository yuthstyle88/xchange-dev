//! #5557 T1 (design `docs/design/org-wide-actor-branch.md` §11, rows C-1..C-6 and G-1..G-3): which users a holder's
//! `BranchScope` covers, and `GrantGuard` reading the same rule. Owner's B-2 answer (#5526, 2026-10-30): a branch
//! ADMIN sees only its own branches' users. Written before the implementation by the ticket author (claude/2ca5874b).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Pure: no database. Roles start at the epoch and never end unless a case says so.
//!
//! Signatures these tests assume (§11.2 [required]): `BranchScope::of_rows(rows) -> BranchScope`,
//! `BranchScope::covers_row(&self, Option<&BranchId>) -> bool`,
//! `BranchScope::covers_user(&self, impl IntoIterator<Item = Option<&BranchId>>) -> bool`.

use chrono::{DateTime, Duration, Utc};
use xc_auth::domain::access::{
    AccountAction, Actor, BranchScope, GrantGuard, GrantRefusal, Role, ScopedRole, TargetUser,
};
use xc_auth::domain::user::UserStatus;
use xc_kernel::ids::BranchId;

const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const B3: &str = "XC-B03";

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

fn b(id: &str) -> BranchId {
    BranchId::new(id)
}

fn branches(ids: &[&str]) -> BranchScope {
    BranchScope::Branches(ids.iter().map(|id| b(id)).collect())
}

fn role(r: Role, branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(r, branch.map(BranchId::new), epoch(), None)
}

/// A role that ended a day after the epoch.
fn ended(r: Role, branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(
        r,
        branch.map(BranchId::new),
        epoch(),
        Some(epoch() + Duration::days(1)),
    )
}

fn actor(id: &str, roles: Vec<ScopedRole>) -> Actor {
    Actor::new(id, "org", roles, None)
}

fn active(id: &str, roles: Vec<ScopedRole>) -> TargetUser {
    TargetUser {
        id: id.to_owned(),
        status: UserStatus::Active,
        roles,
        has_auth_subject: true,
    }
}

fn covers(scope: &BranchScope, rows: &[Option<&str>]) -> bool {
    let rows: Vec<Option<BranchId>> = rows.iter().map(|r| r.map(b)).collect();
    scope.covers_user(rows.iter().map(|r| r.as_ref()))
}

// --- BranchScope (C-1..C-6) ---

#[test]
fn c1_all_covers_every_row_and_every_user_including_one_with_no_live_row() {
    let all = BranchScope::All;
    assert!(all.covers_row(Some(&b(B1))));
    assert!(all.covers_row(None));
    assert!(covers(&all, &[Some(B1), Some(B2)]));
    assert!(covers(&all, &[None]));
    assert!(covers(&all, &[]), "All covers a user with no live row");
}

#[test]
fn c2_branches_cover_a_row_on_a_listed_branch_only() {
    let scope = branches(&[B1]);
    assert!(scope.covers_row(Some(&b(B1))));
    assert!(!scope.covers_row(Some(&b(B2))), "a row on another branch");
    assert!(!scope.covers_row(None), "an organisation-wide row");
}

#[test]
fn c3_branches_cover_a_user_whose_every_live_row_is_on_a_listed_branch() {
    let scope = branches(&[B1, B2]);
    assert!(covers(&scope, &[Some(B1)]));
    assert!(covers(&scope, &[Some(B1), Some(B2)]));
}

#[test]
fn c4_branches_do_not_cover_a_user_with_one_row_elsewhere_or_organisation_wide() {
    let scope = branches(&[B1, B2]);
    assert!(!covers(&scope, &[Some(B1), Some(B3)]));
    assert!(!covers(&scope, &[Some(B1), None]));
    assert!(!covers(&scope, &[None]));
}

#[test]
fn c5_branches_do_not_cover_a_user_with_no_live_row() {
    assert!(!covers(&branches(&[B1]), &[]));
}

#[test]
fn c6_of_rows_is_the_rule_of_of_all_for_an_organisation_wide_row_else_sorted_branches() {
    let rows = [
        role(Role::Admin, Some(B2)),
        role(Role::Admin, Some(B1)),
        role(Role::Admin, Some(B2)),
    ];
    assert_eq!(BranchScope::of_rows(rows.iter()), branches(&[B1, B2]));
    let rows = [role(Role::Admin, Some(B1)), role(Role::Admin, None)];
    assert_eq!(BranchScope::of_rows(rows.iter()), BranchScope::All);
    assert_eq!(BranchScope::of_rows([].iter()), branches(&[]));
}

// --- GrantGuard reads the same rule (G-1..G-3) ---

#[test]
fn g1_a_branch_admin_still_manages_a_user_whose_live_rows_are_all_on_its_branch() {
    let me = actor("a-b01", vec![role(Role::Admin, Some(B1))]);
    let them = active(
        "t",
        vec![role(Role::Teller, Some(B1)), ended(Role::Teller, Some(B2))],
    );
    assert_eq!(
        GrantGuard::check(&me, &them, &AccountAction::Suspend, 2),
        Ok(())
    );
}

#[test]
fn g2_a_branch_admin_does_not_manage_a_user_with_a_live_row_elsewhere() {
    let me = actor("a-b01", vec![role(Role::Admin, Some(B1))]);
    let them = active(
        "t",
        vec![role(Role::Teller, Some(B1)), role(Role::Teller, Some(B2))],
    );
    assert_eq!(
        GrantGuard::check(&me, &them, &AccountAction::Suspend, 2),
        Err(GrantRefusal::NoScope)
    );
}

#[test]
fn g3_a_branch_admin_does_not_manage_a_user_with_no_live_row_but_may_still_grant_one_on_its_branch()
{
    let me = actor("a-b01", vec![role(Role::Admin, Some(B1))]);
    let them = active("t", vec![ended(Role::Teller, Some(B1))]);
    assert_eq!(
        GrantGuard::check(&me, &them, &AccountAction::Suspend, 2),
        Err(GrantRefusal::NoScope),
        "hidden from a branch admin's lists, so not managed by it either (§11.4 G-3)"
    );
    assert_eq!(
        GrantGuard::check(
            &me,
            &them,
            &AccountAction::Grant(role(Role::Teller, Some(B1))),
            2
        ),
        Ok(()),
        "a grant is checked on the granted row's branch, as today"
    );
    let org_admin = actor("a-org", vec![role(Role::Admin, None)]);
    assert_eq!(
        GrantGuard::check(&org_admin, &them, &AccountAction::Suspend, 2),
        Ok(())
    );
}
