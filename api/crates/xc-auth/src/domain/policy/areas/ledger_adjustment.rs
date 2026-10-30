//! LA-0: the policy rows of a FINANCE ledger adjustment (design `docs/design/blueprint/ledger-inventory.md`
//! §13.3). Creating, posting and reading one need `FinanceReconcile`, as `transferReconcile` and F01 do; deciding
//! one is AP-1's `approvalDecide` under `ApproveLedgerAdjustment`, which is not a row here.

use super::{AreaRow, RowScope};
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "ledgerAdjustmentCreate",
        need: Need::Permission(Permission::FinanceReconcile),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "ledgerAdjustmentPost",
        need: Need::Permission(Permission::FinanceReconcile),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "ledgerAdjustments",
        need: Need::Permission(Permission::FinanceReconcile),
        scope: RowScope::Request,
    },
];
