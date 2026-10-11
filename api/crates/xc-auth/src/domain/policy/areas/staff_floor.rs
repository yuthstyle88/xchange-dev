//! Permission rows for the staff floor views (ticket #573, design §3.1).
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use crate::domain::access::Permission;
use crate::domain::policy::areas::AreaRow;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const HOLDS: &[Permission] = &[
    Permission::CustomerHoldPlace,
    Permission::VerificationReview,
];

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "staffMe",
        need: Need::Permission(Permission::SignedIn),
    },
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "staffFloor",
        need: Need::Permission(Permission::AttendanceClock),
    },
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "staffHolds",
        need: Need::AnyOf(HOLDS),
    },
    AreaRow {
        path: EntryPath::StaffRpc,
        action: "staffAttendance",
        need: Need::Permission(Permission::AttendanceClock),
    },
];
