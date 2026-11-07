//! Request and answer shapes of `/api/device-agent/*`, filled by row AG-2 (design §5.4c).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub code: String,
    pub host: String,
    pub os: String,
    pub version: String,
    pub build: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    pub agent_id: String,
    pub token: String,
    pub branch_id: String,
    pub agent_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrawerTargetDto {
    pub transport: String,
    pub target: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRequest {
    pub wait_ms: u64,
    pub version: String,
    pub build: String,
    pub os: String,
    pub host: String,
    #[serde(default)]
    pub drawer: Option<DrawerTargetDto>,
    pub branch_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimResponse {
    pub job_id: String,
    pub lease_id: String,
    pub bytes: String,
    pub send_within_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRequest {
    pub job_id: String,
    pub lease_id: String,
    pub outcome: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportResponse {
    pub recorded: bool,
}

/// `GET /api/device-agent/info` (xc-counter-app.md §8.2).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoResponse {
    pub agent_id: String,
    pub counter_name: String,
    pub branch_id: String,
    pub capabilities: CapabilitiesDto,
}

/// Each setting resolved: not set → drawer off, scanner off, kiosk lock on (§5.2).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesDto {
    pub cash_drawer: bool,
    pub scanner: bool,
    pub kiosk_lock: bool,
}

/// `POST /api/device-agent/desk-binding` (§8.2, §8.3).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeskBindingResponse {
    pub nonce: String,
    /// RFC 3339.
    pub expires_at: String,
}
