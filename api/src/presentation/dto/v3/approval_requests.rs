//! Approval requests DTO (§4.7).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct ApprovalRequestsFilter {
    pub status: Option<String>,
    pub r#type: Option<String>,
}

impl crate::presentation::http::v3::view::ViewFilter for ApprovalRequestsFilter {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct ApprovalDecisionDto {
    pub approver_id: String,
    pub outcome: String,
    pub reason: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct ApprovalRequestItemDto {
    pub id: String,
    pub r#type: String,
    pub branch_id: Option<String>,
    pub object_id: String,
    pub object_version_hash: String,
    pub policy_version: String,
    #[ts(type = "any")]
    pub proposed_payload: Value,
    pub requested_by: String,
    pub requested_at: String,
    pub status: String,
    pub decision: Option<ApprovalDecisionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct ApprovalRequestsData {
    pub requests: Vec<ApprovalRequestItemDto>,
}
