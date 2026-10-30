//! #5558 (design `docs/design/org-wide-actor-branch.md` §12): branch creation, request abandon and the branch list, moved from `Policy::need_table`; `admin_branches` gets its own row in place of the borrowed `adminSnapshot`.

use super::{AreaRow, RowScope};
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const fn row(path: EntryPath, action: &'static str, need: Need) -> AreaRow {
    AreaRow {
        path,
        action,
        need,
        scope: RowScope::Request,
    }
}

const fn org_row(path: EntryPath, action: &'static str, need: Need) -> AreaRow {
    AreaRow {
        scope: RowScope::Organisation,
        ..row(path, action, need)
    }
}

const fn p(permission: Permission) -> Need {
    Need::Permission(permission)
}

pub const ROWS: &[AreaRow] = &[
    org_row(
        EntryPath::Rpc,
        "adminCreateBranch",
        p(Permission::BranchesManage),
    ),
    row(
        EntryPath::Rpc,
        "adminAbandonRequest",
        p(Permission::RequestAbandon),
    ),
    row(EntryPath::Rpc, "admin_branches", p(Permission::CompatRead)),
];
