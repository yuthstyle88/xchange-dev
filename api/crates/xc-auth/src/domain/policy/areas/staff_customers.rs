//! Permission rows for the staff customers view (ticket #580, design §3.1) and the desk's
//! phone verification `customerContactVerify` (BK-10 #1412).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "staffCustomers",
        need: Need::Permission(Permission::CustomerHoldPlace),
    },
    // BK-10 #1412 ruling (a): verifying the customer's phone at the desk.
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "customerContactVerify",
        need: Need::Permission(Permission::CustomerHoldPlace),
    },
];
