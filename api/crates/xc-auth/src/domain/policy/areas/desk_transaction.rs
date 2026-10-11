//! Permission rows for B04 desk transaction reads (ticket #1294, design §4.2).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "receiptReprint",
    need: Need::Permission(Permission::ReceiptReprint),
}];
