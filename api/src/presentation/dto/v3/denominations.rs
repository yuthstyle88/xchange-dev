//! The `denominations` view's wire types (CA-8 #624; lane controller's ruling issuecomment-6107796305): the face
//! values a count or transfer line names by `denominationId`. `faceValue` is the `numeric(20,4)` as a string (W7).
//! The catch-all row (`face_value NULL`) is not countable, so it is not listed (amended ruling
//! issuecomment-6107853376).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One face value of a currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denominations.ts")]
pub struct DenominationRow {
    #[ts(type = "number")]
    pub id: i64,
    pub currency: String,
    pub face_value: String,
    pub group_code: String,
}

/// The `denominations` view: every countable row, ordered by currency, face value, id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denominations.ts")]
pub struct Denominations {
    pub rows: Vec<DenominationRow>,
}
