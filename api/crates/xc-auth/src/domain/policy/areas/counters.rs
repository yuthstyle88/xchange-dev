//! Ticket CA-1d (`docs/design/xc-counter-app.md` §4, §8.4): the policy rows of the branch counters view and save
//! commands.

use super::{AreaRow, RowScope};
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "branchCounters",
        need: Need::Permission(Permission::SettingsRead),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::OwnerRoute,
        action: "branchCounterSave",
        need: Need::Permission(Permission::BranchSettingsSave),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::OwnerRoute,
        action: "branchCashFromBrowserSave",
        need: Need::Permission(Permission::BranchSettingsSave),
        scope: RowScope::Request,
    },
];
