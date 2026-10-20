//! `branch_stock` view DTOs (design `docs/design/blueprint/stock-reservation.md` D-10, D-13, D-17, §4.2 S14, S17, S22).
//!
//! Skeleton by the B05 sub-controller (claude/6530d490) on `feat/b05-stock`. `[required]`: these are the wire
//! shapes the web screen (B05 `/desk/cash`) is cut against.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// D-17: what a `low` row prefills in the existing cash-transfer request (#582). Nothing is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_stock.ts")]
pub struct TransferPrefill {
    pub branch_id: String,
    pub currency: String,
}

/// One currency of the branch (D-10). Quantities are decimal strings, never floats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_stock.ts")]
pub struct BranchStockRow {
    pub currency: String,
    pub on_hand: String,
    pub reserved: String,
    /// D-1: `onHand − reserved`.
    pub available: String,
    /// `branch_stock_floors.min_quantity`; `None` = no floor entered (D-11).
    pub floor: Option<String>,
    /// D-13: `available < floor`; `false` with no floor (S14).
    pub low: bool,
    /// D-17: `Some` exactly when `low`.
    pub transfer_prefill: Option<TransferPrefill>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_stock.ts")]
pub struct BranchStockData {
    pub branch_id: String,
    /// THB first, then A–Z.
    pub rows: Vec<BranchStockRow>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    TransferPrefill::export_all(&cfg)?;
    BranchStockRow::export_all(&cfg)?;
    BranchStockData::export_all(&cfg)?;
    Ok(())
}
