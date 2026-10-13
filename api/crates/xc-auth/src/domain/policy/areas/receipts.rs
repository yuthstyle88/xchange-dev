//! The customer receipt route row (APP-11, ticket #1003).
//!
//! `my_receipt` inherits `(EntryPath::Rpc, "my_receipt")` -> `CustomerSelf`.
//! A customer session is authorized; staff sessions lack `CustomerSelf` and answer `RoleNotAllowedHere`.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "my_receipt",
    need: Need::Permission(Permission::CustomerSelf),
}];
