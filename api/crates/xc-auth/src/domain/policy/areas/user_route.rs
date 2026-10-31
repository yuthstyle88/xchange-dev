//! #5682 (design `docs/design/org-wide-actor-branch.md` §15.12): the `UserRoute` rows, moved from `Policy::need_table` without its catch-all default.

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

pub const ROWS: &[AreaRow] = &[row(
    EntryPath::UserRoute,
    "documents",
    p(Permission::DocumentsList),
)];
