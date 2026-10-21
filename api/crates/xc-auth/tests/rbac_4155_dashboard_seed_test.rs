//! Up-front tests for #4155: the editable-groups seed must not hand TELLER or BRANCH_MANAGER the DASHBOARD cell that
//! `PermissionModule::module` expands into `OverviewRead` (EXECUTIVE only, `areas/executive.rs:2`,
//! `docs/design/blueprint/executive-finance.md:286`) and `AlertSettingsSave`.
//! Written before the implementation by AG controller 4093 (claude/studio-claude-7) on claude/553782's order.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! No database: `Policy::authorize` reads the process snapshot, which is `RoleGrantsSnapshot::seeded()` until
//! something is installed (`rbac_c6a_role_grants_test.rs` `until_something_is_installed_the_process_authorises_from_the_seed`).

mod policy_common;

use policy_common::*;

fn ask(who: &Actor, on: EntryPath, name: &str, named: Option<&str>) -> Result<Grant, PolicyError> {
    Policy::authorize(
        Some(who),
        on,
        &action(name),
        named.map(branch).as_ref(),
        now(),
    )
}

fn holds(r: Role, p: Permission) -> bool {
    RoleGrants::seed(r).contains(&p)
}

#[test]
fn a_seeded_teller_holds_no_dashboard_permission() {
    assert!(
        !holds(Role::Teller, Permission::OverviewRead),
        "OverviewRead is EXECUTIVE only"
    );
    assert!(!holds(Role::Teller, Permission::AlertSettingsSave));
}

#[test]
fn a_seeded_branch_manager_holds_no_dashboard_permission() {
    assert!(
        !holds(Role::BranchManager, Permission::OverviewRead),
        "OverviewRead is EXECUTIVE only"
    );
    assert!(!holds(Role::BranchManager, Permission::AlertSettingsSave));
}

#[test]
fn a_teller_is_refused_the_executive_overview() {
    assert_eq!(
        ask(
            &staff(Role::Teller, "B1"),
            EntryPath::Rpc,
            "executiveOverview",
            None
        )
        .err(),
        Some(PolicyError::RoleNotAllowedHere)
    );
}

#[test]
fn a_teller_is_refused_the_staff_day_report() {
    // TELLER holds none of the readers ShiftCloseCertify, FinanceReconcile, OverviewRead (`areas/staff_report.rs`).
    assert_eq!(
        ask(
            &staff(Role::Teller, "B1"),
            EntryPath::StaffRpc,
            "staffDailyReport",
            Some("B1")
        )
        .err(),
        Some(PolicyError::RoleNotAllowed)
    );
}

#[test]
fn a_branch_manager_is_refused_the_executive_overview() {
    assert_eq!(
        ask(
            &staff(Role::BranchManager, "B1"),
            EntryPath::Rpc,
            "executiveOverview",
            None
        )
        .err(),
        Some(PolicyError::RoleNotAllowedHere)
    );
}

#[test]
fn a_branch_manager_still_reads_the_staff_day_report_through_shift_close_certify() {
    // #4155 row 2: the branch manager keeps the day sheet through ShiftCloseCertify, not OverviewRead.
    assert!(holds(Role::BranchManager, Permission::ShiftCloseCertify));
    let g = ask(
        &staff(Role::BranchManager, "B1"),
        EntryPath::StaffRpc,
        "staffDailyReport",
        Some("B1"),
    )
    .expect("BRANCH_MANAGER reads its own branch's day sheet");
    assert_eq!(g.branch, branch("B1"));
}

#[test]
fn the_executive_keeps_the_overview_and_the_staff_day_report() {
    assert!(holds(Role::Executive, Permission::OverviewRead));
    assert!(holds(Role::Executive, Permission::AlertSettingsSave));
    let exec = staff(Role::Executive, "B1");
    assert!(ask(&exec, EntryPath::Rpc, "executiveOverview", None).is_ok());
    assert!(ask(&exec, EntryPath::StaffRpc, "staffDailyReport", Some("B1")).is_ok());
}

// ── claude/553782's rulings W + A1 (#4155): OverviewRead and AlertSettingsSave for EXECUTIVE only across the seed.
// ADMIN's E-6 AlertSettingsSave waits for #4168, which must settle guardrail 3 (`matrix_guardrails.rs:20-21`).

/// Every built-in staff group but EXECUTIVE.
const NOT_EXECUTIVE: [Role; 7] = [
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
    Role::VerificationReviewer,
];

#[test]
fn only_the_executive_seed_holds_overview_read() {
    let holders: Vec<Role> = RoleGrants::BUILT_IN
        .into_iter()
        .filter(|&r| holds(r, Permission::OverviewRead))
        .collect();
    assert_eq!(holders, vec![Role::Executive]);
}

#[test]
fn only_the_executive_seed_holds_alert_settings_save() {
    let holders: Vec<Role> = RoleGrants::BUILT_IN
        .into_iter()
        .filter(|&r| holds(r, Permission::AlertSettingsSave))
        .collect();
    assert_eq!(holders, vec![Role::Executive]);
}

#[test]
fn a_seeded_admin_holds_no_dashboard_permission() {
    assert!(
        !holds(Role::Admin, Permission::OverviewRead),
        "OverviewRead is EXECUTIVE only"
    );
    assert!(
        !holds(Role::Admin, Permission::AlertSettingsSave),
        "A1: E-6 for ADMIN waits for #4168"
    );
}

#[test]
fn no_group_but_the_executive_passes_the_executive_overview() {
    for r in NOT_EXECUTIVE {
        assert_eq!(
            ask(&staff(r, "B1"), EntryPath::Rpc, "executiveOverview", None).err(),
            Some(PolicyError::RoleNotAllowedHere),
            "{} must be refused executiveOverview",
            r.as_str()
        );
    }
}

#[test]
fn finance_still_reads_the_staff_day_report_through_finance_reconcile() {
    assert!(holds(Role::Finance, Permission::FinanceReconcile));
    let g = ask(
        &staff(Role::Finance, "B1"),
        EntryPath::StaffRpc,
        "staffDailyReport",
        Some("B1"),
    )
    .expect("FINANCE reads the day sheet through FinanceReconcile");
    assert_eq!(g.branch, branch("B1"));
}

#[test]
fn groups_holding_no_day_sheet_reader_are_refused_the_staff_day_report() {
    // None of ShiftCloseCertify, FinanceReconcile, OverviewRead once OverviewRead is EXECUTIVE only.
    for r in [
        Role::Admin,
        Role::RateManager,
        Role::VerificationReviewer,
        Role::Marketing,
    ] {
        assert_eq!(
            ask(
                &staff(r, "B1"),
                EntryPath::StaffRpc,
                "staffDailyReport",
                Some("B1")
            )
            .err(),
            Some(PolicyError::RoleNotAllowed),
            "{} must be refused staffDailyReport",
            r.as_str()
        );
    }
}
