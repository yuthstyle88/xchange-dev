//! Ticket #1003 (APP-11): DTO for view `my_transactions`.
//! Design `docs/design/blueprint/customer-app.md` §3 (C04), §4.4, §9.1.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "my_transactions.ts")]
pub struct MyTransactionsData {
    pub transactions: Vec<MyTransactionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "my_transactions.ts")]
pub struct MyTransactionItem {
    pub id: String,
    pub date: String,
    pub branch: String,
    pub branch_name: String,
    pub r#type: String,
    pub currency: String,
    pub amount: String,
    pub rate: String,
    pub principal: String,
    pub fee: String,
    pub settlement: String,
    pub receipt_ref: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MyTransactionsFilter {
    pub customer_id: Option<String>,
}

pub fn export_types(dir: &std::path::Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MyTransactionsData::export_all(&cfg)?;
    MyTransactionItem::export_all(&cfg)?;
    Ok(())
}
