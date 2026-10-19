//! Ticket #1070 (EX-1): who may export, who may edit the alert thresholds, and which branches an actor may read for
//! a permission. Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/executive-finance.md` (#562) §7.1 (the holders), §7.2 (`BranchScope::of`) and
//! §11.1 EX-1. Owner decisions: E-2 (export by EXECUTIVE and FINANCE), E-6 (EXECUTIVE and ADMIN edit the alert
//! thresholds), E-7 (FINANCE is branch-scoped, possibly several branches).
//!
//! Pure: no database. The time is passed in, never read from the clock.
//!
//! Signatures these tests assume (ticket §7):
//! - `Permission::ReportsExport` and `Permission::AlertSettingsSave` [required].
//! - `xc_auth::domain::access::branch_scope::BranchScope`, `pub enum BranchScope { All, Branches(Vec<BranchId>) }`,
//!   with `Debug + PartialEq` [required]. `Branches` is sorted with no duplicates, and may be empty.
//! - `BranchScope::of(actor: &Actor, permission: Permission, now: DateTime<Utc>) -> BranchScope`, an associated
//!   function [required].
//! - `RoleGrants::seed`, `Actor::new`, `ScopedRole::new` and `Role` as in `authorizer_test.rs`.
//!
//! `Authorizer::check` must answer exactly as before; `authorizer_test.rs` pins that, unchanged.

use chrono::{DateTime, Duration, TimeZone, Utc};
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;

const ORG: &str = "org-a";
const B1: &str = "B1";
const B2: &str = "B2";
const B3: &str = "B3";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 11, 5, 0, 0).unwrap()
}

fn since() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// A row held from the start of 2026 with no end.
fn held(role: Role, at: Option<&str>) -> ScopedRole {
    ScopedRole::new(role, at.map(BranchId::new), since(), None)
}

fn user(roles: Vec<ScopedRole>) -> Actor {
    Actor::new("user-1", ORG, roles, None)
}

fn branches(ids: &[&str]) -> BranchScope {
    BranchScope::Branches(ids.iter().map(|b| BranchId::new(*b)).collect())
}

/// Every role, once. The `match` makes a new `Role` variant a compile error here, so it cannot be left out.
fn every_role() -> Vec<Role> {
    let all = vec![
        Role::Executive,
        Role::Admin,
        Role::RateManager,
        Role::BranchManager,
        Role::Teller,
        Role::Finance,
        Role::Marketing,
        Role::Customer,
        Role::VerificationReviewer,
    ];
    for role in &all {
        match role {
            Role::Executive
            | Role::Admin
            | Role::RateManager
            | Role::BranchManager
            | Role::Teller
            | Role::Finance
            | Role::Marketing
            | Role::Customer
            | Role::VerificationReviewer => {}
        }
    }
    all
}

fn holders(permission: Permission) -> Vec<Role> {
    every_role()
        .into_iter()
        .filter(|role| RoleGrants::seed(*role).contains(&permission))
        .collect()
}

// --- the two new permissions (E-2, E-6) ---

#[test]
fn reports_export_holders_per_section_11_2() {
    // §11.2: ReportsExport (REPORTS FULL) is held by EXECUTIVE, RATE_MANAGER, FINANCE, VERIFICATION_REVIEWER
    assert_eq!(
        holders(Permission::ReportsExport),
        vec![
            Role::Executive,
            Role::RateManager,
            Role::Finance,
            Role::VerificationReviewer,
        ],
        "// §11.2: REPORTS FULL holders"
    );
}

#[test]
fn alert_settings_save_holders_per_section_11_2() {
    // §11.2: AlertSettingsSave (DASHBOARD FULL) is held by EXECUTIVE, RATE_MANAGER, BRANCH_MANAGER, FINANCE, VERIFICATION_REVIEWER
    assert_eq!(
        holders(Permission::AlertSettingsSave),
        vec![
            Role::Executive,
            Role::RateManager,
            Role::BranchManager,
            Role::Finance,
            Role::VerificationReviewer,
        ],
        "// §11.2: DASHBOARD FULL holders"
    );
}

// --- BranchScope::of (§7.2, ticket §8–§9) ---

#[test]
fn an_organisation_wide_executive_reads_all_branches() {
    let actor = user(vec![held(Role::Executive, None)]);
    assert_eq!(
        BranchScope::of(&actor, Permission::OverviewRead, now()),
        BranchScope::All
    );
}

#[test]
fn a_finance_at_two_branches_reads_exactly_those_two() {
    let actor = user(vec![
        held(Role::Finance, Some(B1)),
        held(Role::Finance, Some(B3)),
    ]);
    assert_eq!(
        BranchScope::of(&actor, Permission::FinanceReconcile, now()),
        branches(&[B1, B3])
    );
}

#[test]
fn the_branch_list_is_sorted_and_has_no_duplicates() {
    // Rows given out of order, and B1 twice (two grants at the same branch).
    let actor = user(vec![
        held(Role::Finance, Some(B3)),
        held(Role::Finance, Some(B1)),
        held(Role::Finance, Some(B1)),
    ]);
    assert_eq!(
        BranchScope::of(&actor, Permission::FinanceReconcile, now()),
        branches(&[B1, B3]),
        "§7: Branches sorted, no duplicates"
    );
}

#[test]
fn a_row_that_has_ended_grants_no_branch() {
    let ended = ScopedRole::new(
        Role::Finance,
        Some(BranchId::new(B3)),
        since(),
        Some(now() - Duration::days(1)),
    );
    let actor = user(vec![held(Role::Finance, Some(B1)), ended]);
    assert_eq!(
        BranchScope::of(&actor, Permission::FinanceReconcile, now()),
        branches(&[B1])
    );
}

#[test]
fn a_row_ending_exactly_now_grants_no_branch() {
    // valid_from <= now < valid_to: the end instant itself is outside.
    let ends_now = ScopedRole::new(Role::Finance, Some(BranchId::new(B3)), since(), Some(now()));
    let actor = user(vec![held(Role::Finance, Some(B1)), ends_now]);
    assert_eq!(
        BranchScope::of(&actor, Permission::FinanceReconcile, now()),
        branches(&[B1])
    );
}

#[test]
fn a_row_starting_tomorrow_grants_nothing_yet() {
    let tomorrow = ScopedRole::new(
        Role::Finance,
        Some(BranchId::new(B1)),
        now() + Duration::days(1),
        None,
    );
    let actor = user(vec![tomorrow]);
    assert_eq!(
        BranchScope::of(&actor, Permission::FinanceReconcile, now()),
        branches(&[])
    );
}

#[test]
fn a_role_without_the_permission_reads_no_branch() {
    // TELLER does not hold FinanceReconcile, even at its own branch.
    let actor = user(vec![held(Role::Teller, Some(B1))]);
    assert_eq!(
        BranchScope::of(&actor, Permission::FinanceReconcile, now()),
        branches(&[])
    );
}

#[test]
fn only_rows_holding_the_permission_count() {
    // A FINANCE at B1 and a TELLER at B2: FinanceReconcile reaches B1 only, not the teller's branch.
    let actor = user(vec![
        held(Role::Finance, Some(B1)),
        held(Role::Teller, Some(B2)),
    ]);
    assert_eq!(
        BranchScope::of(&actor, Permission::FinanceReconcile, now()),
        branches(&[B1])
    );
}

#[test]
fn an_organisation_wide_row_wins_over_branch_rows() {
    // §11.2: ADMIN holds DASHBOARD VIEW (OverviewRead), not DASHBOARD FULL (AlertSettingsSave)
    let actor = user(vec![held(Role::Admin, None), held(Role::Admin, Some(B2))]);
    assert_eq!(
        BranchScope::of(&actor, Permission::OverviewRead, now()),
        BranchScope::All
    );
}

#[test]
fn an_organisation_wide_row_without_the_permission_does_not_make_all() {
    // An org-wide MARKETING row does not hold FinanceReconcile, so it must not turn a FINANCE at B1 into All.
    let actor = user(vec![
        held(Role::Marketing, None),
        held(Role::Finance, Some(B1)),
    ]);
    assert_eq!(
        BranchScope::of(&actor, Permission::FinanceReconcile, now()),
        branches(&[B1])
    );
}

#[test]
fn an_ended_organisation_wide_row_does_not_make_all() {
    let ended_everywhere = ScopedRole::new(
        Role::Executive,
        None,
        since(),
        Some(now() - Duration::days(1)),
    );
    let actor = user(vec![ended_everywhere, held(Role::Finance, Some(B1))]);
    assert_eq!(
        BranchScope::of(&actor, Permission::ReportsExport, now()),
        branches(&[B1]),
        "the EXECUTIVE row has ended; only the FINANCE row at B1 holds ReportsExport now"
    );
}

#[test]
fn an_actor_with_no_roles_reads_no_branch() {
    let actor = user(vec![]);
    assert_eq!(
        BranchScope::of(&actor, Permission::OverviewRead, now()),
        branches(&[])
    );
}
