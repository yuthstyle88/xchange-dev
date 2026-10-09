//! Admin reports view DTOs for v3 (§7, §12.1).

use actix_web::http::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::presentation::dto::v3::common::Period;

use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::ViewFilter;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct ReportPeriod {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct ReportStats {
    pub received: String,
    pub paid: String,
    pub fees: String,
    pub bank_fees: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct ReportChannel {
    pub method: String,
    #[ts(type = "number")]
    pub count: u64,
    pub received: String,
    pub paid: String,
    pub net: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct ReportCurrency {
    pub currency: String,
    pub buy_amount: String,
    pub sell_amount: String,
    pub principal: String,
    pub fees: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct ReportReference {
    #[ts(type = "number")]
    pub referenced: u64,
    #[ts(type = "number")]
    pub unreferenced: u64,
    pub value: String,
    pub spread: String,
    pub percent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct ReportEmployeeCurrency {
    pub currency: String,
    pub received: String,
    pub paid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct ReportEmployeeGroup {
    pub staff: String,
    #[ts(type = "number")]
    pub count: u64,
    #[ts(type = "number")]
    pub completed: u64,
    #[ts(type = "number")]
    pub pending: u64,
    #[ts(type = "number")]
    pub cancelled: u64,
    pub received: String,
    pub paid: String,
    pub net: String,
    pub fees: String,
    pub bank_fees: String,
    pub methods: Vec<ReportChannel>,
    pub currencies: Vec<ReportEmployeeCurrency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct ReportEmployeeTotals {
    #[ts(type = "number")]
    pub people: u64,
    #[ts(type = "number")]
    pub completed: u64,
    pub received: String,
    pub paid: String,
    pub net: String,
}

/// Answer data for the `admin_reports` view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct AdminReportsData {
    pub period: ReportPeriod,
    pub stats: ReportStats,
    pub by_channel: Vec<ReportChannel>,
    pub by_currency: Vec<ReportCurrency>,
    pub reference: ReportReference,
    pub by_employee: Vec<ReportEmployeeGroup>,
    pub employee_totals: ReportEmployeeTotals,
}

/// Filter for the `admin_reports` view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export_to = "admin_reports.ts")]
pub struct AdminReportsFilter {
    pub from: String,
    pub to: String,
    pub staff: String,
    pub staff_q: String,
}

impl ViewFilter for AdminReportsFilter {
    fn check(&self) -> Result<(), V3Error> {
        Period {
            from: self.from.clone(),
            to: self.to.clone(),
        }
        .check()?;

        if self.staff_q.chars().count() > 100 {
            return Err(V3Error::new(
                "invalidRequest",
                Some("staffQ"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            ));
        }

        Ok(())
    }
}
