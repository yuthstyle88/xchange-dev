//! The `rates_draftable` view's wire types (`docs/design/board-currencies.md` D-8, §4): what a rate draft can add.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One currency of the branch's set a draft can carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rates_draftable.ts")]
pub struct DraftableCurrency {
    pub code: String,
    pub group: String,
    /// The register's unit, as a decimal string.
    pub unit_size: String,
    /// The newest BoT batch has a line for it.
    pub has_reference: bool,
}

/// The branch's `SHOW` and `ASK` rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rates_draftable.ts")]
pub struct RatesDraftableData {
    pub rows: Vec<DraftableCurrency>,
}
