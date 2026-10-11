//! TX-4 (#672): the policy rows of the two exchange settings commands (`quote-transaction.md`
//! §4.2; lane controller ruling on #672, 2026-10-10).
//!
//! The organisation's quote lifetime is ADMIN's (T1, `SystemSettings`); a branch's approval limit
//! is `BranchesManage` (A04), not the exchange permission a teller holds.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "exchangeSettingsSave",
        need: Need::Permission(Permission::SystemSettings),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "branchExchangeLimitSave",
        need: Need::Permission(Permission::BranchesManage),
    },
];
