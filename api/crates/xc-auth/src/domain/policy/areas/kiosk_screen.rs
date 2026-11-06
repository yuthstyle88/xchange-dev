//! #6273: the policy rows of the kiosk screen values' view and save. Read by every staff group (`SettingsRead`);
//! saved by EXECUTIVE only (`SystemSettings`, seed v10), as every business setting is (§11.11).

use super::{AreaRow, RowScope};
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "kioskScreen",
        need: Need::Permission(Permission::SettingsRead),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "kioskScreenSave",
        need: Need::Permission(Permission::SystemSettings),
        scope: RowScope::Request,
    },
];
