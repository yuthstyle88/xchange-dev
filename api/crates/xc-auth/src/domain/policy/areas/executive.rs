//! EX-3 (#1071): the policy row of A01, the organisation-wide executive overview
//! (`docs/design/blueprint/executive-finance.md` §7.1). `OverviewRead` is held by EXECUTIVE only.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "executiveOverview",
    need: Need::Permission(Permission::OverviewRead),
}];
