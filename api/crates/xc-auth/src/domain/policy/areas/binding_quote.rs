//! `binding-quote.md` D-13 (O-1 A): who may issue a binding quote. Only `BindingQuoteIssue`, which the built-in
//! TELLER and BRANCH_MANAGER groups hold; `ExchangeTransact` is not enough (it maps to Customers at Create).

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "bindingQuoteIssue",
    need: Need::Permission(Permission::BindingQuoteIssue),
}];
