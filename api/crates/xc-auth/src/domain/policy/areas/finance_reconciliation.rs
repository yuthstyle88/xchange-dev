//! FN-1 (#1068): the policy rows of F01, FINANCE's reconcile-and-certify sheet
//! (`docs/design/blueprint/executive-finance.md` §5, §8.2). Both need `FinanceReconcile`, which FINANCE holds.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "financeReconciliation",
        need: Need::Permission(Permission::FinanceReconcile),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "reconciliationCertify",
        need: Need::Permission(Permission::FinanceReconcile),
    },
];
