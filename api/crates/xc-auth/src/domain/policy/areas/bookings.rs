//! BK-6 (#1388): policy rows for booking commands and views.
//!
//! (docs/design/blueprint/booking-slots.md §4.4, §9.1)

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const CUSTOMER_OR_DESK: &[Permission] = &[Permission::CustomerSelf, Permission::BookingsDesk];

pub const ROWS: &[AreaRow] = &[
    // Commands
    AreaRow {
        path: EntryPath::Rpc,
        action: "bookingCreate",
        need: Need::AnyOf(CUSTOMER_OR_DESK),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "bookingCheckIn",
        need: Need::Permission(Permission::BookingsDesk),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "kioskBookingCheckIn",
        need: Need::Public,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "bookingConfirm",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "bookingCancel",
        need: Need::AnyOf(CUSTOMER_OR_DESK),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "bookingReschedule",
        need: Need::AnyOf(CUSTOMER_OR_DESK),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "bookingQrReissue",
        need: Need::AnyOf(CUSTOMER_OR_DESK),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "slotRuleSet",
        need: Need::Permission(Permission::BranchesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "closureAdd",
        need: Need::Permission(Permission::BranchesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "closureRemove",
        need: Need::Permission(Permission::BranchesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "slotCapacitySet",
        need: Need::Permission(Permission::BranchesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "slotClose",
        need: Need::Permission(Permission::BranchesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "slotOpen",
        need: Need::Permission(Permission::BranchesManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "bookingPolicySet",
        need: Need::Permission(Permission::BranchesManage),
    },
    // Views
    AreaRow {
        path: EntryPath::Rpc,
        action: "booking_slots",
        need: Need::Public,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "desk_bookings",
        need: Need::Permission(Permission::BookingsDesk),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "my_bookings",
        need: Need::Permission(Permission::CustomerSelf),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "admin_booking_slots",
        need: Need::Permission(Permission::BranchesManage),
    },
];
