//! SP-1 (#1585, rates design §12.6, §12.10): policy rows for org starting prices.
//!
//! Entering and reading org starting prices needs `OrgStartingPricesManage` (EXECUTIVE only).

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "orgStartingPriceSave",
        need: Need::Permission(Permission::OrgStartingPricesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "orgStartingPrices",
        need: Need::Permission(Permission::OrgStartingPricesManage),
    },
];
