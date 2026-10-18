//! OR-5a (#945): the policy rows of the A04 branch and A05 user commands and views.
//!
//! Scaffold written by the ticket author: `ROWS` is empty; the implementation adds, each on
//! `EntryPath::Rpc`: `branchSave` and `adminBranchesProfile` → `BranchesManage`; `userGrant`,
//! `userRevoke`, `userDisable` and `adminUsers` → `UsersManage`. No invite (owner, 2026-10-10);
//! `userCreate` is #1157's.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const fn row(action: &'static str, permission: Permission) -> AreaRow {
    AreaRow {
        path: EntryPath::Rpc,
        action,
        need: Need::Permission(permission),
    }
}

pub const ROWS: &[AreaRow] = &[
    row("branchSave", Permission::BranchesManage),
    row("adminBranchesProfile", Permission::BranchesManage),
    row("userGrant", Permission::UsersManage),
    row("userRevoke", Permission::UsersManage),
    row("userDisable", Permission::UsersManage),
    row("adminUsers", Permission::UsersManage),
    row("userSuspend", Permission::UsersManage),
    row("userResume", Permission::UsersManage),
    row("userEnable", Permission::UsersManage),
    row("userResetPassword", Permission::UsersManage),
    row("userProfileSave", Permission::UsersManage),
    row("adminPermissionMatrix", Permission::UsersManage),
    row("adminUserAudit", Permission::AuditLogsRead),
    row("adminLoginHistory", Permission::AuditLogsRead),
];
