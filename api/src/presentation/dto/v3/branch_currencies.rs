//! The `branch_currencies` view's wire types (`docs/design/board-currencies.md` §4): which currencies a staff form
//! may list. The `denominations` view says which notes each has.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One currency a staff form may list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_currencies.ts")]
pub struct BranchCurrencyRow {
    pub code: String,
    pub group: String,
    /// `"SHOW"` · `"ASK"` · `"HIDE"`. A currency held with no row reads `"HIDE"`.
    pub display: String,
    pub sort_order: i32,
    /// The branch holds this currency now.
    pub held: bool,
}

/// The branch's rows, then any currency it holds with no row, then THB if it has neither.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_currencies.ts")]
pub struct BranchCurrenciesData {
    pub rows: Vec<BranchCurrencyRow>,
}
