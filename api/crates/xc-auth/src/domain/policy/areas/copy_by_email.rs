//! KWQ slice d (design `kiosk-shared-phone.md` §4.2 D21): the member's e-mail copy of their own
//! booking or web request. The public link page (`POST /api/public/copy-link`) has no policy row,
//! like every public read (`presentation/http/public/mod.rs`).

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "copyByEmail",
    need: Need::Permission(Permission::CustomerSelf),
}];
