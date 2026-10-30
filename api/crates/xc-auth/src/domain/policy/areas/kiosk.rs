//! #5558 (design `docs/design/org-wide-actor-branch.md` §12): the kiosk's routes, moved from `Policy::need_table` (R-8, owner Q12, `kiosk-shared-phone.md` §4.2 D19: only staff issue numbers, so `confirm` needs `QueueOperate`).

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
    row(EntryPath::Kiosk, "rates", Need::Public),
    row(EntryPath::Kiosk, "quote", Need::Public),
    row(EntryPath::Kiosk, "status", Need::Public),
    row(EntryPath::Kiosk, "customerReceipt", Need::Public),
    row(EntryPath::Kiosk, "customerCancel", Need::Public),
    row(EntryPath::Kiosk, "settings", Need::Public),
    row(EntryPath::Kiosk, "manage", p(Permission::RatesDraft)),
    row(EntryPath::Kiosk, "configure", p(Permission::RatesDraft)),
    row(
        EntryPath::Kiosk,
        "configureSettings",
        p(Permission::RatesDraft),
    ),
    row(EntryPath::Kiosk, "notifications", p(Permission::SignedIn)),
    row(EntryPath::Kiosk, "list", p(Permission::SignedIn)),
    row(EntryPath::Kiosk, "confirm", p(Permission::QueueOperate)),
    row(
        EntryPath::Kiosk,
        "identityChecked",
        p(Permission::QueueOperate),
    ),
    row(
        EntryPath::Kiosk,
        "publishReceipt",
        p(Permission::QueueOperate),
    ),
    row(EntryPath::Kiosk, "get", p(Permission::QueueOperate)),
    row(EntryPath::Kiosk, "accept", p(Permission::QueueOperate)),
    row(EntryPath::Kiosk, "cancel", p(Permission::QueueOperate)),
    row(
        EntryPath::Kiosk,
        "issueByToken",
        p(Permission::QueueOperate),
    ),
    row(EntryPath::Kiosk, "findByToken", p(Permission::QueueOperate)),
];
