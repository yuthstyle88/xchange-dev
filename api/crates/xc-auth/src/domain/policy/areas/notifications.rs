use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "notificationRead",
        need: Need::Permission(Permission::SignedIn),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "notificationReadAll",
        need: Need::Permission(Permission::SignedIn),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "myNotifications",
        need: Need::Permission(Permission::SignedIn),
    },
];

