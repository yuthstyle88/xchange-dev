//! K8 (design `customer-identity-consent.md` §4.4): `identityReveal`, the route
//! `POST /api/customers/{id}/identities/{identityId}/reveal`, needs `IdentityDocumentRead`.
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::{AreaRow, RowScope};
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "identityReveal",
    need: Need::Permission(Permission::IdentityDocumentRead),
    scope: RowScope::Request,
}];
