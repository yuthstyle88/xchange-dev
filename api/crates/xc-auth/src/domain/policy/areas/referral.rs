//! RA-3 (#981): the policy rows of the referral commands and view (O-2, owner 2026-10-10:
//! "Teller attaches, OWNER replaces").
//!
//! Attaching a code needs `ReferralAttach` (TELLER and EXECUTIVE); replacing an ACTIVE one needs
//! `ReferralSupersede` (EXECUTIVE only); reading the attributions needs `ReferralRead` (EXECUTIVE
//! and TELLER, scoped to the teller's branch by the seam).

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "attachReferral",
        need: Need::Permission(Permission::ReferralAttach),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "supersedeReferral",
        need: Need::Permission(Permission::ReferralSupersede),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "referralAttributions",
        need: Need::Permission(Permission::ReferralRead),
    },
];
