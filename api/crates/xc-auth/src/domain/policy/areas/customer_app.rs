//! The customer app's planner rows (APP-4 #994, design `customer-app.md` §4.4).
//!
//! The draft commands, the preview and the drafts view are `Public` on the route table: `Need` has
//! no "a customer or an anonymous app session" row, so the owner is checked by `PlannerService`
//! against the `DraftOwner` the route builds (the actor's customer, else the `xc_app_sid` session).

use super::AreaRow;
use crate::domain::access::permission::Permission;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "plannerDraftSave",
        need: Need::Public,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "plannerDraftDelete",
        need: Need::Public,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "planner_preview",
        need: Need::Public,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "my_planner_drafts",
        need: Need::Public,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "appSettingsSave",
        need: Need::Permission(Permission::SystemSettings),
    },
];
