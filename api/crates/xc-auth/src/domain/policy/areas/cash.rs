//! Permission rows for the cash commands and views (ticket #624 CA-8; the lane controller's G2 and G3 rulings on #624,
//! design `cash-shift-transfer.md` §3.1, §4.4, §4.7). Every row is on `EntryPath::Rpc` (G2a). A row is the union of
//! what the service itself checks, so the service's more precise refusal is still reached.
//!
//! Picked up by `xc-auth/build.rs` to generate `AREA_ROWS`.

use crate::domain::access::Permission;
use crate::domain::policy::areas::AreaRow;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const COUNT_SUBMIT: &[Permission] = &[Permission::ShiftOperate, Permission::CashManage];
const TRANSFER_RECEIVE: &[Permission] = &[Permission::CashManage, Permission::ShiftOperate];
const CASH_COUNTS: &[Permission] = &[
    Permission::ShiftOperate,
    Permission::ShiftCloseCertify,
    Permission::CashManage,
];

pub const ROWS: &[AreaRow] = &[
    // commands (§4.4, G3)
    AreaRow {
        path: EntryPath::Rpc,
        action: "countSubmit",
        need: Need::AnyOf(COUNT_SUBMIT),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "countVerify",
        need: Need::Permission(Permission::ShiftCloseCertify),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "handoverStart",
        need: Need::Permission(Permission::ShiftOperate),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "handoverAccept",
        need: Need::Permission(Permission::ShiftOperate),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "handoverReject",
        need: Need::Permission(Permission::ShiftOperate),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "transferCreate",
        need: Need::Permission(Permission::CashManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "transferSubmit",
        need: Need::Permission(Permission::CashManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "transferDispatch",
        need: Need::Permission(Permission::CashManage),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "transferReceive",
        need: Need::AnyOf(TRANSFER_RECEIVE),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "transferReconcile",
        need: Need::Permission(Permission::FinanceReconcile),
    },
    // views (§4.7)
    AreaRow {
        path: EntryPath::Rpc,
        action: "myShift",
        need: Need::Permission(Permission::ShiftOperate),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "handoversPending",
        need: Need::Permission(Permission::ShiftOperate),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "cashCounts",
        need: Need::AnyOf(CASH_COUNTS),
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "cashTransfers",
        need: Need::Permission(Permission::CashManage),
    },
];
