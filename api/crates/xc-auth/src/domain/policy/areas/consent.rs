//! #5558 (design `docs/design/org-wide-actor-branch.md` §12): consent texts and retention, moved from `Policy::need_table`.

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
        "consentPolicy",
        p(Permission::ConsentPolicyManage),
    ),
    row(EntryPath::Rpc, "consentTexts", p(Permission::SettingsRead)),
];
