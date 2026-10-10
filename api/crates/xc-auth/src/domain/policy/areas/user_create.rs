//! Who may create a staff member (#1157): `POST /api/users` (`userCreate`). The shop owner or an
//! ADMIN creates the person's Identity account server-side; nobody signs up from outside.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "userCreate",
    need: Need::Permission(Permission::UsersManage),
}];
