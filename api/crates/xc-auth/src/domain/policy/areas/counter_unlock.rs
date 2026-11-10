//! Policy row for `counterUnlockIssue` (xchange#6569, `docs/design/xc-counter-app.md` §8.8).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::{AreaRow, RowScope};
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "counterUnlockIssue",
    need: Need::Permission(Permission::CounterUnlock),
    scope: RowScope::Request,
}];
