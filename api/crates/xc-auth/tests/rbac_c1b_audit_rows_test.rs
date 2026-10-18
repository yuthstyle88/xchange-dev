//! RBAC C1b up-front test (design `docs/design/blueprint/user-management-rbac.md`, PR 3083, §3.3,
//! §3.6 row 3): the two read-only history views need C2's `Permission::AuditLogsRead`.
//! Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or
//! weaken; add tests in a file of your own.
//! Names `Permission::AuditLogsRead`, which C2 adds: it fails to compile until C2 merges, then by
//! result until C1b adds the rows. No database.

use xc_auth::domain::access::Permission;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;

#[test]
fn the_audit_and_login_history_views_need_audit_logs_read() {
    for name in ["adminUserAudit", "adminLoginHistory"] {
        assert_eq!(
            Policy::need(EntryPath::Rpc, &Action::new(name)),
            Some(Need::Permission(Permission::AuditLogsRead)),
            "{name}"
        );
    }
}
