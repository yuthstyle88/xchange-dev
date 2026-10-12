//! Desk refunds DTO for the `desk_refunds` view (ticket #1265, DK-K).
//! Design: `docs/design/xchange-v3-wave3-desk.md` §5.5.2.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::reversal::ReversalSummaryDto;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_refunds.ts")]
pub struct DeskRefundLine {
    pub shop_side: String,
    pub currency: String,
    pub quantity: String,
    pub settle_thb: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_refunds.ts")]
pub struct DeskRefundPayment {
    pub method: String,
    pub amount_thb: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_refunds.ts")]
pub struct DeskRefundRow {
    pub reversal: ReversalSummaryDto,
    pub receipt_number: String,
    pub lines: Vec<DeskRefundLine>,
    pub payments: Vec<DeskRefundPayment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "desk_refunds.ts")]
pub struct DeskRefunds {
    pub rows: Vec<DeskRefundRow>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DeskRefundLine::export_all(&cfg)?;
    DeskRefundPayment::export_all(&cfg)?;
    DeskRefundRow::export(&cfg)?;
    DeskRefunds::export(&cfg)?;
    Ok(())
}
