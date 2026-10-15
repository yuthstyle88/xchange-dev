//! Permission rows for customer verification (ticket #1399, design §4.4, §9.1 row VF-8).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "my_verification",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationCaseOpen",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationCaseSubmit",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationDocumentRemove",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationMyDocumentLink",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationDocumentUpload",
        need: Need::Permission(Permission::CustomerSelf),
    },
];
