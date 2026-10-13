//! The customer history view row (APP-11, ticket #1003).
//!
//! `my_transactions` inherits `(EntryPath::Rpc, "my_transactions")` -> `CustomerSelf`.
//! A customer session is authorized; staff sessions lack `CustomerSelf` and answer `RoleNotAllowedHere`.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "my_transactions",
    need: Need::Permission(Permission::CustomerSelf),
}];
