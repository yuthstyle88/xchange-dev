//! The customer app's assistant rows (APP-6 #997, design `customer-app.md` §4.4).
//!
//! The assistantAsk command and the my_assistant_conversation view are `Public` on the route table:
//! `Need` has no "a customer or an anonymous app session" row, so the owner is checked by
//! `AssistantService` against the `DraftOwner` the route builds (the actor's customer, else the
//! `xc_app_sid` session).

use super::AreaRow;
use crate::domain::policy::entities::policy::Need;
use crate::domain::policy::value_objects::entry_path::EntryPath;

pub const ROWS: &[AreaRow] = &[
    AreaRow {
        path: EntryPath::Rpc,
        action: "assistantAsk",
        need: Need::Public,
    },
    AreaRow {
        path: EntryPath::Rpc,
        action: "my_assistant_conversation",
        need: Need::Public,
    },
];
