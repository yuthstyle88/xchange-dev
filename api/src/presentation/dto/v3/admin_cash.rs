//! Admin cash view DTOs for v3 (§7, §12.1, §12.6, §12.9).

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::OpenShiftDto;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// Filter keys accepted when drilling down from executive overview (G2, ruling on #1106).
pub const DRILLDOWN_KEYS: &[&str] = &["all", "currency"];

/// Filter for the `admin_cash` view (`all=true` includes all 153 register currencies).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashFilter {
    pub all: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub currency: Option<String>,
}

impl ViewFilter for AdminCashFilter {
    fn check(&self) -> Result<(), V3Error> {
        if let Some(ref c) = self.currency
            && (c.len() != 3 || !c.bytes().all(|b| b.is_ascii_uppercase()))
        {
            return Err(V3Error::new(
                "invalidRequest",
                Some("currency"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }
        Ok(())
    }
}

/// Stock row per currency shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashStockRow {
    pub code: String,
    pub opening: String,
    pub net: String,
    pub expected: String,
}

/// Cash flow stats in THB for the open shift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashStats {
    pub opening: String,
    pub incoming: String,
    pub outgoing: String,
    pub remaining: String,
}

/// Reference spread summary over the open shift's exchanges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashReference {
    pub with_reference: u32,
    pub without_reference: u32,
    pub spread: String,
    pub percent: String,
}

/// Counted amount per currency from the last closed shift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashCountedItem {
    pub code: String,
    pub amount: String,
}

/// Answer data for the `admin_cash` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_cash.ts")]
pub struct AdminCashData {
    pub open_shifts: Vec<OpenShiftDto>,
    pub stock: Vec<AdminCashStockRow>,
    pub stats: Option<AdminCashStats>,
    pub reference: AdminCashReference,
    pub currencies: Vec<String>,
    pub last_counted: Option<Vec<AdminCashCountedItem>>,
}
