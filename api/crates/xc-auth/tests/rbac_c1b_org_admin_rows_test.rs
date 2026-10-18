//! RBAC C1b up-front tests (design `docs/design/blueprint/user-management-rbac.md`, PR 3083, §3.6
//! row 3): the policy rows of the new user commands and views, landed before the F tickets write
//! their handlers. Rows naming C2's `AuditLogsRead` are in `rbac_c1b_audit_rows_test.rs`.
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or
//! weaken; add tests in a file of your own.
//! Compiles on `origin/main` and fails there by result (`Policy::need` answers `None`). No database.

use xc_auth::domain::access::Permission;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

const USERS_MANAGE_ROWS: [&str; 6] = [
    "userSuspend",
    "userResume",
    "userEnable",
    "userResetPassword",
    "userProfileSave",
    "adminPermissionMatrix",
];

#[test]
fn every_new_user_command_and_the_matrix_view_need_users_manage_on_rpc() {
    let mut wrong = Vec::new();
    for name in USERS_MANAGE_ROWS {
        let got = Policy::need(EntryPath::Rpc, &Action::new(name));
        if got != Some(Need::Permission(Permission::UsersManage)) {
            wrong.push(format!("{name}: {got:?}"));
        }
    }
    assert_eq!(
        wrong,
        Vec::<String>::new(),
        "rows that are missing or wrong"
    );
}

#[test]
fn the_existing_org_admin_rows_are_unchanged() {
    for (name, want) in [
        ("branchSave", Permission::BranchesManage),
        ("adminBranchesProfile", Permission::BranchesManage),
        ("userGrant", Permission::UsersManage),
        ("userRevoke", Permission::UsersManage),
        ("userDisable", Permission::UsersManage),
        ("adminUsers", Permission::UsersManage),
    ] {
        assert_eq!(
            Policy::need(EntryPath::Rpc, &Action::new(name)),
            Some(Need::Permission(want)),
            "{name}"
        );
    }
}

#[test]
fn the_new_rows_are_rpc_only() {
    for name in USERS_MANAGE_ROWS {
        assert_eq!(
            Policy::need(EntryPath::Kiosk, &Action::new(name)),
            None,
            "{name} on the kiosk path"
        );
    }
}
