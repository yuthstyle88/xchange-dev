//! Shift valuation compute action DTOs for v3 (§12.20.4).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::{Money, Rate};

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "shift_valuation.ts")]
pub struct ShiftValuationRequest {
    pub shift_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rates: Option<BTreeMap<String, String>>, // absent or null: list the currencies only
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "shift_valuation.ts")]
pub struct ShiftValuation {
    pub currencies: Vec<ShiftValuationCurrency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result: Option<ShiftValuationResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "shift_valuation.ts")]
pub struct ShiftValuationCurrency {
    pub currency: String,
    pub opening: Money,
    pub current: Money,
    pub needs_rate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "shift_valuation.ts")]
pub struct ShiftValuationResult {
    pub rows: Vec<ShiftValuationRow>, // one per entry of `currencies`, same order
    pub opening: Money,
    pub current: Money,
    pub movements: Money,
    pub difference: Money,
    pub adjusted: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "shift_valuation.ts")]
pub struct ShiftValuationRow {
    pub currency: String,
    pub rate: Rate,
    pub value: Money,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ShiftValuationRequest::export_all(&cfg)?;
    ShiftValuation::export_all(&cfg)?;
    ShiftValuationCurrency::export_all(&cfg)?;
    ShiftValuationResult::export_all(&cfg)?;
    ShiftValuationRow::export_all(&cfg)?;
    Ok(())
}
