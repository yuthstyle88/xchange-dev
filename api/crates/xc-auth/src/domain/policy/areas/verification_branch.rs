//! Permission rows for VF-12 at-branch intake (ticket #1398, design §5, §9).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use super::AreaRow;
use crate::domain::access::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::Rpc,
    action: "verificationBranchCaseOpen",
    need: Need::Permission(Permission::VerificationReview),
}];
