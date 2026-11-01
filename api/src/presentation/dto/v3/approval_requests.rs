//! Approval requests DTO (§4.7).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_kernel::ids::BranchId;

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct ApprovalRequestsFilter {
    pub status: Option<String>,
    pub r#type: Option<String>,
}

impl crate::presentation::http::v3::view::ViewFilter for ApprovalRequestsFilter {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct ApprovalDecisionDto {
    pub approver_id: String,
    pub approver_name: Option<String>,
    pub outcome: String,
    pub reason: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "common.ts")]
pub struct ReversalSubjectLine {
    pub side: String,
    pub currency: String,
    pub quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
/// In `common.ts`: both `approval_requests` and `finance_reconciliation` carry it (`v3_types_test`).
#[ts(export_to = "common.ts")]
pub struct ReversalSubject {
    pub reversal_id: String,
    pub original_transaction_id: String,
    pub receipt_number: String,
    pub amount_thb: String,
    pub reason: String,
    pub customer_name: Option<String>,
    pub customer_no: Option<String>,
    pub requested_by_name: Option<String>,
    pub shift_id: Option<String>,
    pub shift_opened_at: Option<String>,
    pub lines: Vec<ReversalSubjectLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct CountSubjectLine {
    pub currency: String,
    pub counted: String,
    pub expected: Option<String>,
    pub difference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct CountDifferenceSubject {
    pub count_id: String,
    pub count_kind: String,
    pub counted_by: String,
    pub counted_by_name: Option<String>,
    pub shift_id: Option<String>,
    pub shift_opened_at: Option<String>,
    pub net_difference_thb: String,
    pub lines: Vec<CountSubjectLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub enum ApprovalSubject {
    Reversal(ReversalSubject),
    CountDifference(CountDifferenceSubject),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct ApprovalRequestItemDto {
    pub id: String,
    pub r#type: String,
    pub branch_id: Option<String>,
    pub object_id: String,
    pub object_version_hash: String,
    pub policy_version: String,
    #[ts(type = "any")]
    pub proposed_payload: Value,
    pub requested_by: String,
    pub requested_by_name: Option<String>,
    pub requested_at: String,
    pub status: String,
    pub decision: Option<ApprovalDecisionDto>,
    pub subject: Option<ApprovalSubject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "approval_requests.ts")]
pub struct ApprovalRequestsData {
    pub requests: Vec<ApprovalRequestItemDto>,
}

fn money(mut d: Decimal) -> String {
    d.rescale(2);
    d.to_string()
}

pub async fn load_reversal_subjects(
    pool: &PgPool,
    branch: &BranchId,
    ids: &[Uuid],
) -> AppResult<HashMap<Uuid, ReversalSubject>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = sqlx::query(
        "SELECT rv.id, rv.original_transaction_id, COALESCE(t.document_ref, '') AS receipt_number, \
                rv.amount_thb, rv.reason, \
                NULLIF(btrim(c.display_name), '') AS customer_name, \
                NULLIF(btrim(c.customer_no), '') AS customer_no, \
                NULLIF(btrim(u.name), '') AS requested_by_name, \
                t.shift_id, sh.opened_at AS shift_opened_at \
           FROM reversals rv \
           JOIN transactions t ON t.id = rv.original_transaction_id \
           LEFT JOIN customers c ON c.id = t.customer_id \
           LEFT JOIN users u ON u.id::text = rv.requested_by \
           LEFT JOIN shifts sh ON sh.id = t.shift_id \
          WHERE rv.id = ANY($1) \
            AND rv.branch_id = $2",
    )
    .bind(ids)
    .bind(branch.as_str())
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    if rows.is_empty() {
        return Ok(HashMap::new());
    }

    let tx_ids: Vec<Uuid> = rows
        .iter()
        .map(|r| r.get("original_transaction_id"))
        .collect();

    let line_rows = sqlx::query(
        "SELECT transaction_id, shop_side::text AS side, currency, quantity \
           FROM transaction_lines \
          WHERE transaction_id = ANY($1) \
          ORDER BY transaction_id, line_no ASC",
    )
    .bind(&tx_ids)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    let mut lines_by_tx: HashMap<Uuid, Vec<ReversalSubjectLine>> = HashMap::new();
    for lr in line_rows {
        let tx_id: Uuid = lr.get("transaction_id");
        let side: String = lr.get("side");
        let currency: String = lr.get("currency");
        let quantity: Decimal = lr.get("quantity");
        lines_by_tx
            .entry(tx_id)
            .or_default()
            .push(ReversalSubjectLine {
                side,
                currency: currency.trim().to_string(),
                quantity: quantity.to_string(),
            });
    }

    let mut out = HashMap::with_capacity(rows.len());
    for r in rows {
        let id: Uuid = r.get("id");
        let original_tx_id: Uuid = r.get("original_transaction_id");
        let receipt_number: String = r.get("receipt_number");
        let amount_thb: Decimal = r.get("amount_thb");
        let reason: String = r.get("reason");
        let customer_name: Option<String> = r.get("customer_name");
        let customer_no: Option<String> = r.get("customer_no");
        let requested_by_name: Option<String> = r.get("requested_by_name");
        let shift_id: Option<String> = r.get("shift_id");
        let shift_opened_at: Option<DateTime<Utc>> = r.get("shift_opened_at");

        out.insert(
            id,
            ReversalSubject {
                reversal_id: id.to_string(),
                original_transaction_id: original_tx_id.to_string(),
                receipt_number,
                amount_thb: money(amount_thb),
                reason,
                customer_name,
                customer_no,
                requested_by_name,
                shift_id,
                shift_opened_at: shift_opened_at.map(|dt| dt.to_rfc3339()),
                lines: lines_by_tx
                    .get(&original_tx_id)
                    .cloned()
                    .unwrap_or_default(),
            },
        );
    }

    Ok(out)
}

pub async fn load_count_difference_subjects(
    pool: &PgPool,
    branch: &BranchId,
    request_rows: &[(Uuid, Value)],
) -> AppResult<HashMap<Uuid, CountDifferenceSubject>> {
    if request_rows.is_empty() {
        return Ok(HashMap::new());
    }

    let count_ids: Vec<Uuid> = request_rows.iter().map(|(id, _)| *id).collect();
    let payload_by_count: HashMap<Uuid, &Value> =
        request_rows.iter().map(|(id, p)| (*id, p)).collect();

    let rows = sqlx::query(
        "SELECT c.id, c.kind, c.counted_by::text AS counted_by, \
                NULLIF(btrim(u.name), '') AS counted_by_name, \
                c.shift_id, sh.opened_at AS shift_opened_at \
           FROM cash_counts c \
           LEFT JOIN users u ON u.id = c.counted_by \
           LEFT JOIN shifts sh ON sh.id = c.shift_id \
          WHERE c.id = ANY($1) \
            AND c.branch_id = $2",
    )
    .bind(&count_ids)
    .bind(branch.as_str())
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    if rows.is_empty() {
        return Ok(HashMap::new());
    }

    let total_rows = sqlx::query(
        "SELECT count_id, currency, counted, expected, difference \
           FROM cash_count_totals \
          WHERE count_id = ANY($1) \
          ORDER BY count_id, currency ASC",
    )
    .bind(&count_ids)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    let mut lines_by_count: HashMap<Uuid, Vec<CountSubjectLine>> = HashMap::new();
    for tr in total_rows {
        let count_id: Uuid = tr.get("count_id");
        let currency: String = tr.get("currency");
        let counted: Decimal = tr.get("counted");
        let expected: Option<Decimal> = tr.get("expected");
        let difference: Option<Decimal> = tr.get("difference");
        lines_by_count
            .entry(count_id)
            .or_default()
            .push(CountSubjectLine {
                currency: currency.trim().to_string(),
                counted: counted.to_string(),
                expected: expected.map(|d| d.to_string()),
                difference: difference.map(|d| d.to_string()),
            });
    }

    let mut out = HashMap::with_capacity(rows.len());
    for r in rows {
        let id: Uuid = r.get("id");
        let count_kind: String = r.get("kind");
        let counted_by: String = r.get("counted_by");
        let counted_by_name: Option<String> = r.get("counted_by_name");
        let shift_id: Option<String> = r.get("shift_id");
        let shift_opened_at: Option<DateTime<Utc>> = r.get("shift_opened_at");
        let net_difference_thb = payload_by_count
            .get(&id)
            .and_then(|p| p.get("net_difference_thb"))
            .and_then(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| v.as_f64().map(|n| format!("{n:.2}")))
            })
            .unwrap_or_default();

        out.insert(
            id,
            CountDifferenceSubject {
                count_id: id.to_string(),
                count_kind,
                counted_by,
                counted_by_name,
                shift_id,
                shift_opened_at: shift_opened_at.map(|dt| dt.to_rfc3339()),
                net_difference_thb,
                lines: lines_by_count.remove(&id).unwrap_or_default(),
            },
        );
    }

    Ok(out)
}
