//! MK-4 (#3402, marketing-campaigns.md §4.5): the `margin_guard` view's answer.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The `margin_guard` view's answer (`GET /api/v3/views/margin_guard`).
/// All three fields are `null` when the organisation has no `margin_guards` row, or when a field is `NULL`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "margin_guard.ts", rename_all = "camelCase")]
pub struct MarginGuardData {
    pub max_spread_share: Option<String>,
    pub set_by: Option<String>,
    pub set_at: Option<String>,
}
