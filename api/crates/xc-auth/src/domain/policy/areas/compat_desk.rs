//! #5558 (design `docs/design/org-wide-actor-branch.md` §12): the v2 desk and admin snapshot reads and the desk's sale, moved from `Policy::need_table`.

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
    row(EntryPath::Rpc, "adminSnapshot", p(Permission::CompatRead)),
    row(EntryPath::Rpc, "adminDeskConfig", p(Permission::CompatRead)),
    row(
        EntryPath::Rpc,
        "getConnectionStatus",
        p(Permission::CompatRead),
    ),
    row(
        EntryPath::Rpc,
        "reserveCustomer",
        p(Permission::ExchangeTransact),
    ),
    row(
        EntryPath::Rpc,
        "saveTransaction",
        p(Permission::ExchangeTransact),
    ),
    row(
        EntryPath::StaffRpc,
        "adminSnapshot",
        p(Permission::CompatRead),
    ),
    row(
        EntryPath::StaffRpc,
        "getConnectionStatus",
        p(Permission::CompatRead),
    ),
    row(
        EntryPath::StaffRpc,
        "adminDeskConfig",
        p(Permission::ExchangeTransact),
    ),
    row(
        EntryPath::StaffRpc,
        "reserveCustomer",
        p(Permission::ExchangeTransact),
    ),
    row(
        EntryPath::StaffRpc,
        "saveTransaction",
        p(Permission::ExchangeTransact),
    ),
    row(
        EntryPath::StaffRpc,
        "documents",
        p(Permission::IdentityDocumentRead),
    ),
    row(EntryPath::Rpc, "me", p(Permission::SignedIn)),
];
