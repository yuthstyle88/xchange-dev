//! The `handover_receivers` view's wire types (CA-8 #624; lane controller's ruling issuecomment-6107853376): who the
//! caller may hand their drawer to.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A person who could accept the hand-over: their user id and name, nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "handover_receivers.ts")]
pub struct HandoverReceiverRow {
    pub user_id: String,
    pub name: String,
}

/// The `handover_receivers` view, ordered by name, then user id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "handover_receivers.ts")]
pub struct HandoverReceivers {
    pub rows: Vec<HandoverReceiverRow>,
}
