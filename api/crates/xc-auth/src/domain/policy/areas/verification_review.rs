//! Permission rows for verification review (ticket #1396, design §4.5, §9.1 row VF-7).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "verification_queue",
        need: Need::Permission(Permission::VerificationReview),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verification_case",
        need: Need::Permission(Permission::VerificationReview),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationCaseStartReview",
        need: Need::Permission(Permission::VerificationReview),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationCaseApprove",
        need: Need::Permission(Permission::VerificationReview),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationCaseReject",
        need: Need::Permission(Permission::VerificationReview),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationCaseRequestUpdate",
        need: Need::Permission(Permission::VerificationReview),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationCaseRevoke",
        need: Need::Permission(Permission::VerificationReview),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "verificationDocumentLink",
        need: Need::Permission(Permission::VerificationReview),
    },
];
