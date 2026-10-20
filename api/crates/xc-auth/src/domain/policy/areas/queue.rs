//! QU-7 (#1266): the policy rows of the queue's v3 commands and its board (design
//! `docs/design/blueprint/queue-tickets.md` §4.4, `:151-162`).
//!
//! Named as §4.4's table names them (the lane controller's ruling on #1266,
//! issuecomment-6107392168): the desk's commands and the board need `QueueOperate` (TELLER and
//! BRANCH_MANAGER); opening and closing the queue need `BranchDayOperate` (BRANCH_MANAGER).
//! `branch_service_status` (public, C08) and `my_queue_ticket` (the ticket's own customer) have
//! no row: neither is a staff route.

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

const fn queue_operate(action: &'static str) -> AreaRow {
    AreaRow {
        path: EntryPath::Rpc,
        action,
        need: Need::Permission(Permission::QueueOperate),
    }
}

const fn branch_day(action: &'static str) -> AreaRow {
    AreaRow {
        path: EntryPath::Rpc,
        action,
        need: Need::Permission(Permission::BranchDayOperate),
    }
}

pub const ROWS: &[AreaRow] = &[
    queue_operate("ticketIssueWalkIn"),
    queue_operate("ticketCallNext"),
    queue_operate("ticketCall"),
    queue_operate("ticketRecall"),
    queue_operate("ticketSkip"),
    queue_operate("ticketStart"),
    queue_operate("ticketFinish"),
    queue_operate("ticketCancel"),
    queue_operate("counterPause"),
    queue_operate("counterResume"),
    queue_operate("queueBoard"),
    branch_day("queueOpen"),
    branch_day("queueClose"),
    AreaRow {
        path: EntryPath::Rpc,
        action: "queueTicketTake",
        need: Need::Permission(Permission::CustomerSelf),
    },
];
