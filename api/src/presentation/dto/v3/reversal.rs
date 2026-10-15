//! RV-2 (#682): the answer of `POST /api/transactions/{id}/reversal-requests` (design `reversal.md` §4.2). A
//! command DTO: no TS is committed (finding RG7 on #682).
//! RV-4 (#685): ReversalSummaryDto for the reversal view and B04 transaction view.

use std::path::Path;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_exchange::domain::reversal::reversal::Reversal;
use xc_exchange::domain::reversal::window::{ReversalWindow, Tz};

/// camelCase; money as a 2-place string, the business date as `YYYY-MM-DD`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReversalDto {
    pub id: String,
    pub status: String,
    pub original_transaction_id: String,
    pub reason: String,
    pub amount_thb: String,
    pub business_date: String,
    pub approval_request_id: String,
}

impl ReversalDto {
    pub fn from_domain(r: &Reversal) -> Self {
        Self {
            id: r.id.0.to_string(),
            status: r.status.as_str().to_owned(),
            original_transaction_id: r.original_transaction_id.0.to_string(),
            reason: r.reason.clone(),
            amount_thb: r.amount_thb.to_string(),
            business_date: r.business_date.to_string(),
            approval_request_id: r.approval_request_id.to_string(),
        }
    }
}

/// Filter for the `reversal` view (RV-4 #685).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "reversal.ts")]
pub struct ReversalFilter {
    pub id: Option<String>,
}

impl crate::presentation::http::v3::view::ViewFilter for ReversalFilter {}

/// Reversal summary DTO for both reads (view `reversal` and B04 field on `transaction`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "reversal.ts")]
pub struct ReversalSummaryDto {
    pub id: String,
    pub original_transaction_id: String,
    pub status: String,
    pub reason: String,
    pub amount: String,
    pub requested_by: String,
    pub requested_at: String,
    pub approved_at: Option<String>,
    pub posted_at: Option<String>,
    pub document_number: Option<String>,
}

/// Complete response data for the `reversal` view.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "reversal.ts")]
pub struct ReversalViewData {
    pub reversal: Option<ReversalSummaryDto>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ReversalFilter::export_all(&cfg)?;
    ReversalSummaryDto::export_all(&cfg)?;
    ReversalViewData::export_all(&cfg)?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ReversalBy {
    Id(Uuid),
    Original(Uuid),
}

/// O-7: Single loader and reversals-row -> ReversalSummaryDto mapping for views.
pub(crate) async fn load_reversal_summary(
    pool: &PgPool,
    by: ReversalBy,
    branch_id: &str,
) -> AppResult<Option<ReversalSummaryDto>> {
    let (reversal_id, original_tx_id) = match by {
        ReversalBy::Id(id) => (Some(id), None),
        ReversalBy::Original(id) => (None, Some(id)),
    };
    let row = sqlx::query(
        "SELECT r.id, r.original_transaction_id, r.status, r.reason, r.amount_thb, \
         r.requested_by, r.requested_at, r.business_date, b.timezone, \
         ad.at AS approved_at, r.posted_at, rc.number AS document_number \
         FROM reversals r \
         JOIN branches b ON b.id = r.branch_id \
         LEFT JOIN approval_decisions ad ON ad.request_id = r.approval_request_id AND ad.outcome = 'APPROVED' \
         LEFT JOIN receipts rc ON rc.id = r.receipt_id \
         WHERE ($1::uuid IS NULL OR r.id = $1) \
           AND ($2::uuid IS NULL OR r.original_transaction_id = $2) \
           AND r.branch_id = $3 \
         ORDER BY r.requested_at DESC, r.created_at DESC \
         LIMIT 1",
    )
    .bind(reversal_id)
    .bind(original_tx_id)
    .bind(branch_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::Database)?;

    let Some(row) = row else {
        return Ok(None);
    };

    map_reversal_summary_row(row, Utc::now()).map(Some)
}

pub(crate) async fn load_reversal_summaries(
    pool: &PgPool,
    ids: &[Uuid],
    branch_id: &str,
) -> AppResult<Vec<ReversalSummaryDto>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT r.id, r.original_transaction_id, r.status, r.reason, r.amount_thb, \
         r.requested_by, r.requested_at, r.business_date, b.timezone, \
         ad.at AS approved_at, r.posted_at, rc.number AS document_number \
         FROM reversals r \
         JOIN branches b ON b.id = r.branch_id \
         LEFT JOIN approval_decisions ad ON ad.request_id = r.approval_request_id AND ad.outcome = 'APPROVED' \
         LEFT JOIN receipts rc ON rc.id = r.receipt_id \
         WHERE r.id = ANY($1) \
           AND r.branch_id = $2 \
         ORDER BY r.requested_at ASC, r.created_at ASC",
    )
    .bind(ids)
    .bind(branch_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    let now = Utc::now();
    rows.into_iter()
        .map(|row| map_reversal_summary_row(row, now))
        .collect()
}

fn map_reversal_summary_row(
    row: sqlx::postgres::PgRow,
    now: DateTime<Utc>,
) -> AppResult<ReversalSummaryDto> {
    let id: Uuid = row.get("id");
    let original_id: Uuid = row.get("original_transaction_id");
    let stored_status: String = row.get("status");
    let reason: String = row.get("reason");
    let amount_thb: Decimal = row.get("amount_thb");
    let requested_by: String = row.get("requested_by");
    let requested_at: DateTime<Utc> = row.get("requested_at");
    let business_date: NaiveDate = row.get("business_date");
    let timezone: String = row.get("timezone");
    let approved_at: Option<DateTime<Utc>> = row.get("approved_at");
    let posted_at: Option<DateTime<Utc>> = row.get("posted_at");
    let document_number: Option<String> = if stored_status == "POSTED" {
        row.get("document_number")
    } else {
        None
    };

    let tz = Tz::from_str(&timezone)
        .map_err(|e| AppError::Internal(format!("invalid timezone {timezone}: {e}")))?;
    let status = if stored_status == "APPROVED" && !ReversalWindow::is_open(business_date, now, tz)
    {
        "EXPIRED".to_string()
    } else {
        stored_status
    };

    Ok(ReversalSummaryDto {
        id: id.to_string(),
        original_transaction_id: original_id.to_string(),
        status,
        reason,
        amount: format!("{:.2}", amount_thb),
        requested_by,
        requested_at: requested_at.to_rfc3339(),
        approved_at: approved_at.map(|t| t.to_rfc3339()),
        posted_at: posted_at.map(|t| t.to_rfc3339()),
        document_number,
    })
}
