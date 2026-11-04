//! Backdated keying (`docs/design/backdated-keying.md` §3): `transactionBackdate`, the Owner's and Branch Manager's
//! entry of an exchange at its real past time. Its own permission, so a teller is refused at the seam (rule R1).

use super::{AreaRow, RowScope};
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "transactionBackdate",
    need: Need::Permission(Permission::ExchangeBackdate),
    scope: RowScope::Request,
}];
