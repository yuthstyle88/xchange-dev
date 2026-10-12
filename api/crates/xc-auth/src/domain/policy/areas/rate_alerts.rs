//! RA-2b (#1429): policy rows for rate alerts commands and views.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "rateAlertCreate",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "rateAlertPause",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "rateAlertResume",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "rateAlertDelete",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "rateAlertSettingsSave",
        need: Need::Permission(Permission::RateAlertSettingsSave),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "myRateAlerts",
        need: Need::Permission(Permission::SignedIn),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "rateAlertSettings",
        need: Need::Permission(Permission::RateAlertSettingsSave),
    },
];
