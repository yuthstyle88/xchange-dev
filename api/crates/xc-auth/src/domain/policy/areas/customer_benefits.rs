//! Permission row for the customer benefits view (C05 #3433, design `marketing-campaigns.md` §6.3, O-3).

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "my_benefits",
    need: Need::Permission(Permission::CustomerSelf),
}];
