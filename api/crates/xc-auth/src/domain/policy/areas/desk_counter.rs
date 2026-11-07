//! Policy row for `deskBindCounter` (CA-1c, `docs/design/xc-counter-app.md` §5.4, §8.4).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::{AreaRow, RowScope};
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "deskBindCounter",
    need: Need::Permission(Permission::AttendanceClock),
    scope: RowScope::Request,
}];
