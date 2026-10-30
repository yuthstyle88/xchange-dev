//! Permission rows for the branch-day commands (ticket #1274 CA-8b, design `cash-shift-transfer.md` §3.2, §4.4, §9 CA-8b).
//! Every row is on `EntryPath::Rpc`.
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use crate::domain::access::Permission;
use crate::domain::policy::areas::{AreaRow, RowScope};
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "dayStartService",
        need: Need::Permission(Permission::BranchDayOperate),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "dayCutoff",
        need: Need::Permission(Permission::BranchDayOperate),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "dayTarget",
        need: Need::Permission(Permission::CashTargetSet),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "clock",
        need: Need::Permission(Permission::AttendanceClock),
        scope: RowScope::Request,
    },
    // Staff corrections §4 A: a manager closes another employee's forgotten clock-in, and lists them.
    AreaRow {
        path: EntryPath::Rpc,
        action: "attendanceCloseFor",
        need: Need::Permission(Permission::AttendanceCloseFor),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "openAttendance",
        need: Need::Permission(Permission::AttendanceCloseFor),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "holdCreate",
        need: Need::Permission(Permission::CustomerHoldPlace),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "holdRelease",
        need: Need::Permission(Permission::VerificationReview),
        scope: RowScope::Request,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "draftBegin",
        need: Need::Permission(Permission::ShiftOperate),
        scope: RowScope::Request,
    },
];
