//! Permission row for the staff customers view (ticket #580, design §3.1).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::StaffRpc,
    action: "staffCustomers",
    need: Need::Permission(Permission::CustomerHoldPlace),
}];
