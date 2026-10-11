//! EX-5 (#1075): the policy rows of the branch alert settings view and save command.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "branchAlertSettings",
        need: Need::Permission(Permission::AlertSettingsSave),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "branchAlertSettingsSave",
        need: Need::Permission(Permission::AlertSettingsSave),
    },
];
