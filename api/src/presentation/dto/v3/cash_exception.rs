//! DTOs for `cashExceptionReview` and `cashExceptionRaise` (#3413, section B S1).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_exception_review.ts")]
pub struct CashExceptionReviewInput {
    pub branch: String,
    pub exception_id: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_exception_review.ts")]
pub struct CashExceptionReviewOutput {
    pub exception_id: String,
    pub status: String,
    pub reviewed_by: String,
    pub reviewed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_exception_raise.ts")]
pub struct CashExceptionDifferenceInput {
    pub currency: String,
    pub amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_exception_raise.ts")]
pub struct CashExceptionRaiseInput {
    pub branch: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub place: String,
    #[serde(default)]
    pub differences: Vec<CashExceptionDifferenceInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_exception_raise.ts")]
pub struct CashExceptionRaiseOutput {
    pub exception_id: String,
    pub status: String,
}
