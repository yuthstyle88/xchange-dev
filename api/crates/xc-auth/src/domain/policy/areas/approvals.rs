//! #5558 (design `docs/design/org-wide-actor-branch.md` §12): approval decisions and reads, and reversal requests, moved from `Policy::need_table`.

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

const fn p(permission: Permission) -> Need {
    Need::Permission(permission)
}

pub const ROWS: &[AreaRow] = &[
    row(
        EntryPath::Rpc,
        "approvalDecide",
        p(Permission::ApprovalsDecide),
    ),
    row(
        EntryPath::Rpc,
        "approvalRead",
        p(Permission::ApprovalsDecide),
    ),
    row(
        EntryPath::Rpc,
        "reversalRequest",
        p(Permission::ReversalRequest),
    ),
];
