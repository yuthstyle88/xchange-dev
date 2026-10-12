//! #1350: the staff report, `GET /api/v3/views/staff_daily_report?branch=&day=` (design
//! `xchange-v3-wave3-staff-report.md` §B.1, §B.3), camelCase. Money and quantities are decimal strings (W7); counts are
//! JSON numbers. The screen does no arithmetic: every figure here is the server's.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The view's query key besides `branch`: the business date `YYYY-MM-DD`; none = today in the branch's timezone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_daily_report.ts")]
pub struct StaffDailyReportFilter {
    #[serde(default)]
    #[ts(optional)]
    pub day: Option<String>,
}

/// The day's state (B.1 row 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_daily_report.ts")]
pub enum ReportStatus {
    Reconciled,
    ClosedAwaitingReconciliation,
    Preliminary,
}

/// One of v2's seven currencies (B.1 row 13): the drawer columns sum the day's verified CLOSE counts (Q-SR1), the
/// safe columns are the day's last verified safe count; `null` where no count gives the figure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_daily_report.ts")]
pub struct ReportCurrencyRow {
    pub currency: String,
    pub drawer_counted: Option<String>,
    pub drawer_expected: Option<String>,
    pub drawer_difference: Option<String>,
    pub safe_counted: Option<String>,
    pub safe_expected: Option<String>,
    pub safe_difference: Option<String>,
}

/// A note of the day (Q-SR3): the record's own kind code and its note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_daily_report.ts")]
pub struct ReportNote {
    pub kind: String,
    pub note: String,
}

/// Money in transit (Q-SR4): one DISPATCHED transfer's amount in one currency, Σ `qty_sent × face_value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_daily_report.ts")]
pub struct ReportTransit {
    pub kind: String,
    pub amount: String,
    pub currency: String,
}

/// What a signer did on a closing count (D-B5): the capacity, not an access role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_daily_report.ts")]
pub enum SignerCapacity {
    Counted,
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_daily_report.ts")]
pub struct ReportSigner {
    pub name: String,
    pub capacity: SignerCapacity,
}

/// The report (B.3 `StaffDailyReportData`). No customer is named in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "staff_daily_report.ts")]
pub struct StaffDailyReportData {
    /// The business date shown, `YYYY-MM-DD`.
    pub day: String,
    /// The branch's IANA timezone, which bounds the day and formats `generatedAt`.
    pub timezone: String,
    /// When the server answered, RFC 3339.
    pub generated_at: String,
    pub company: Option<String>,
    pub site: Option<String>,
    pub branch_name: String,
    pub status: ReportStatus,
    #[ts(type = "number")]
    pub completed: u64,
    pub buy_volume: String,
    pub sell_volume: String,
    pub reversed_volume: String,
    #[ts(type = "number")]
    pub pending_transactions: u64,
    pub currencies: Vec<ReportCurrencyRow>,
    #[ts(type = "number")]
    pub pending_requests: u64,
    #[ts(type = "number")]
    pub open_exceptions: u64,
    pub notes: Vec<ReportNote>,
    #[ts(type = "number")]
    pub open_holds: u64,
    pub in_transit: Vec<ReportTransit>,
    pub signers: Vec<ReportSigner>,
    pub finance_note: Option<String>,
}
