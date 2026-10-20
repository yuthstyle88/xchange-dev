//! The answers of the two exchange settings commands (design `quote-transaction.md` §4.2, §3.6).
//! Keys `[required]` (lane controller ruling on #672, issuecomment-6104304961): the camelCase of
//! `quote_ttl_seconds` / `approval_limit_thb`; money is a decimal string (D-4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `PUT /api/exchange-settings`: the organisation's settings after the change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "exchange_settings.ts")]
pub struct ExchangeSettingsDto {
    pub quote_ttl_seconds: i32,
}

/// `PUT /api/branches/{id}/exchange-limit`: the branch's limit after the change; `null` is no
/// limit, so no exchange waits (T2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "exchange_settings.ts")]
pub struct BranchExchangeLimitDto {
    pub branch_id: String,
    pub approval_limit_thb: Option<String>,
}

/// `GET /api/v3/views/exchange_settings`: the organisation's quote lifetime and the branch's
/// approval limit (`null` = no limit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "exchange_settings.ts")]
pub struct ExchangeSettingsViewData {
    pub quote_ttl_seconds: i32,
    pub approval_limit_thb: Option<String>,
}
