//! The staff report's policy row (ticket #1350, design `xchange-v3-wave3-staff-report.md` D-B2): the §02 role scopes
//! that read a branch's day sheet. `ShiftCloseCertify` (BRANCH_MANAGER: cash and shift close), `FinanceReconcile`
//! (FINANCE: reconciliation) and `OverviewRead` (EXECUTIVE: every branch). A session holding none of them is refused
//! `RoleNotAllowed` before the view loads. Staff path only: the owner's cookie alone is not a staff session.
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use crate::domain::access::Permission;
use crate::domain::policy::areas::AreaRow;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const READERS: &[Permission] = &[
    Permission::ShiftCloseCertify,
    Permission::FinanceReconcile,
    Permission::OverviewRead,
];

pub const ROWS: &[AreaRow] = &[AreaRow {
    path: EntryPath::StaffRpc,
    action: "staffDailyReport",
    need: Need::AnyOf(READERS),
}];
