//! The `transfer_branches` view's wire types (CA-8 #624; lane controller's rulings issuecomment-6107955194 and A on
//! issuecomment-6107959194): where a BRANCH transfer may go.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A branch a transfer may be sent to: its id and name, nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transfer_branches.ts")]
pub struct TransferBranchRow {
    pub branch_id: String,
    pub name: String,
}

/// The `transfer_branches` view, ordered by name, then branch id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transfer_branches.ts")]
pub struct TransferBranches {
    pub rows: Vec<TransferBranchRow>,
}
