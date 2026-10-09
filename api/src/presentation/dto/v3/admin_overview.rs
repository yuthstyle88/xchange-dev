//! Admin overview view DTOs for v3 (§7, §12.1, §12.2).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::Period;
use crate::presentation::dto::v3::transaction_row::TransactionRow;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

/// Filter for the `admin_overview` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_overview.ts")]
pub struct AdminOverviewFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<String>,
}

impl ViewFilter for AdminOverviewFilter {
    fn check(&self) -> Result<(), V3Error> {
        let period = Period {
            from: self.from.clone().unwrap_or_default(),
            to: self.to.clone().unwrap_or_default(),
        };
        period.check()
    }
}

/// Totals computed over the period's LIVE completed exchanges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_overview.ts")]
pub struct AdminOverviewTotals {
    pub volume: String,
    pub buy_principal: String,
    pub sell_principal: String,
    pub received: String,
    pub paid: String,
    pub net: String,
    pub fees: String,
    pub buy_share: String,
    pub sell_share: String,
    pub buy_count: u32,
    pub sell_count: u32,
    pub completed_count: u32,
    pub cancelled_count: u32,
    pub pending_count: u32,
}

/// Status strip figures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_overview.ts")]
pub struct AdminOverviewStatus {
    pub shift_open: bool,
    pub pending_count: u32,
    pub opened_by: Option<String>,
}

/// Complete response data for `admin_overview`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_overview.ts")]
pub struct AdminOverviewData {
    pub period: Period,
    pub totals: AdminOverviewTotals,
    pub latest: Vec<TransactionRow>,
    pub status: AdminOverviewStatus,
}
