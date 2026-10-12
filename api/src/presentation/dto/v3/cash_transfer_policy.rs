//! CA-8c (#1371): transfer policy wire types (`PUT /api/cash-transfer-policy` and `cash_transfer_policy` view).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The answer of `transferPolicySave` and the `cash_transfer_policy` view's `data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_transfer_policy.ts")]
pub struct CashTransferPolicyData {
    pub version: String,
    #[ts(type = "Record<string, string>")]
    pub limits: BTreeMap<String, String>,
    pub set_by: String,
    pub created_at: String,
}

/// The body of `PUT /api/cash-transfer-policy`, beside the seam's `branch`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_transfer_policy.ts")]
pub struct TransferPolicySaveInput {
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
    #[ts(type = "Record<string, string>")]
    pub limits: BTreeMap<String, String>,
}

/// The `cash_transfer_policy` view query parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "cash_transfer_policy.ts")]
pub struct CashTransferPolicyFilter {
    #[serde(default)]
    #[ts(optional)]
    pub branch: Option<String>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CashTransferPolicyData::export_all(&cfg)?;
    CashTransferPolicyFilter::export_all(&cfg)?;
    TransferPolicySaveInput::export_all(&cfg)?;
    Ok(())
}
