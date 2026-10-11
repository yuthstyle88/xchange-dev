//! CA-0p (#1272): the policy row of the cash transfer policy save command.
//!
//! Setting a cash-transfer limit policy needs `CashTransferPolicySet` (EXECUTIVE only,
//! cash-shift-transfer blueprint §3.1, §9).

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "transferPolicySave",
    need: Need::Permission(Permission::CashTransferPolicySet),
}];
