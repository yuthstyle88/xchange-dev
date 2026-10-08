//! Device transactions view DTOs for v3 (§7.4).

use serde::Serialize;
use ts_rs::TS;

use super::common::Money;

/// A row in the `device_transactions` page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "device_transactions.ts")]
pub struct DeviceTransactionRow {
    pub id: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub transaction_type: String,
    pub amount: Money,
    pub currency: String,
}
