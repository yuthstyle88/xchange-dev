//! Payment methods DTOs (PM-API).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// View returned by `POST /api/payment-methods` on `list` and `save`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentMethodsView {
    #[serde(rename = "branchId")]
    pub branch_id: String,
    pub version: i64,
    pub methods: Value,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
    #[serde(rename = "updatedBy")]
    pub updated_by: Option<String>,
    #[serde(rename = "canEdit")]
    pub can_edit: bool,
    pub history: Vec<PaymentAuditEntry>,
}

/// A single history entry in `PaymentMethodsView`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentAuditEntry {
    pub version: i64,
    pub actor: String,
    pub created_at: String,
    pub before_methods: Value,
    pub after_methods: Value,
}
