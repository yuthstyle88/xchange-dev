//! The `my_web_request` view DTOs (design `kiosk-shared-phone.md` §4.2 D8).

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_web_request.ts")]
pub struct MyWebRequestBranch {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_web_request.ts")]
pub struct MyWebRequestData {
    pub request_id: String,
    pub status: String,
    pub expires_at: Option<String>,
    pub branch: MyWebRequestBranch,
    #[serde(rename = "type")]
    pub kind: String,
    pub currency: String,
    pub amount: String,
    pub display_number: Option<String>,
    pub tickets_ahead: Option<i32>,
}
