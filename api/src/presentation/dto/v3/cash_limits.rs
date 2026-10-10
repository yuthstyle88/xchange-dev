//! LG-3b (#926, ledger design §3.3): a branch's count-difference limit (`cash_limits`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The `cash_limits` view's answer, and what `cashLimitsSave` answers after a save.
/// `countDifferenceLimitThb` is a decimal string with two places, or `null` (no limit: every
/// non-zero difference goes to FINANCE). A branch with no row answers `null` everywhere but
/// `branch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_limits.ts")]
pub struct CashLimitsData {
    pub branch: String,
    pub count_difference_limit_thb: Option<String>,
    pub updated_by: Option<String>,
    pub updated_at: Option<String>,
}

impl CashLimitsData {
    /// The answer for `branch` from its `cash_limits` row, read as
    /// `(count_difference_limit_thb::text, updated_by, updated_at)`; `None` is a branch with no
    /// row. The save and the view both answer through this, so they answer the same object.
    pub fn of(
        branch: &str,
        row: Option<(Option<String>, String, chrono::DateTime<chrono::Utc>)>,
    ) -> Self {
        match row {
            Some((limit, by, at)) => Self {
                branch: branch.to_owned(),
                count_difference_limit_thb: limit,
                updated_by: Some(by),
                updated_at: Some(at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
            },
            None => Self {
                branch: branch.to_owned(),
                count_difference_limit_thb: None,
                updated_by: None,
                updated_at: None,
            },
        }
    }
}

/// The body of `PUT /api/cash-limits`, beside the seam's `branch`.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_limits_save.ts")]
pub struct CashLimitsSaveInput {
    pub count_difference_limit_thb: Option<String>,
}
