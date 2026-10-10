//! LG-3b (#926): the policy rows of the cash limits command and view.
//!
//! Setting and reading a branch's `cash_limits` needs `CashLimitsManage` (FINANCE and ADMIN,
//! ledger design §11 Q-2), not `CashManage`: a branch manager must not raise the limit that decides
//! when FINANCE checks their own branch's counts.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "cashLimitsSave",
        need: Need::Permission(Permission::CashLimitsManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "cashLimitsRead",
        need: Need::Permission(Permission::CashLimitsManage),
    },
];
