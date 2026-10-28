//! LA-7: `GET /api/v3/views/ledger_adjustments?branch=&status=` (design `docs/design/blueprint/ledger-inventory.md`
//! §13.5 "Read"): the branch's FINANCE ledger adjustments, policy row `ledgerAdjustments` (`FinanceReconcile`).
//! One read joining `approval_requests` (type `LEDGER_ADJUSTMENT`, `proposed_payload->>'branch'` = the branch,
//! never `approval_requests.branch_id`, §13.3), its latest `approval_decisions` row and `ledger_postings` on
//! `object_id = source_id` (`source_type 'ADJUSTMENT'`), with the valuations from the posting's SAFE entries.
//! A `status` other than empty, `PENDING`, `APPROVED`, `REJECTED` or `POSTED` is `400 invalidRequest`.
//!
//! Not versioned: a decision runs under the decider's branch lock (`approval_decide.rs`), not the adjustment's,
//! so it need not move this branch's version. The screen reads it on demand; nothing polls it.
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): `load` is LA-7's.

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::ledger_adjustment_create::{
    LedgerAdjustmentEvidence, LedgerAdjustmentLine,
};
use crate::presentation::dto::v3::ledger_adjustment_post::LedgerAdjustmentPostedLine;
use crate::presentation::dto::v3::ledger_adjustments::{
    LedgerAdjustmentRow, LedgerAdjustments, LedgerAdjustmentsFilter,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint, ViewFilter};

impl ViewFilter for LedgerAdjustmentsFilter {
    fn check(&self) -> Result<(), V3Error> {
        match self.status.as_str() {
            "" | "PENDING" | "APPROVED" | "REJECTED" | "POSTED" => Ok(()),
            _ => Err(V3Error::new(
                "invalidRequest",
                Some("status"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )),
        }
    }
}

struct LedgerAdjustmentsView {
    pool: PgPool,
}

impl BranchView for LedgerAdjustmentsView {
    const NAME: &'static str = "ledger_adjustments";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ledgerAdjustments");
    const VERSIONED: bool = false;
    type Data = LedgerAdjustments;
    type Filter = LedgerAdjustmentsFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        let who = who.clone();
        let status_filter = filter.status.clone();

        async move {
            let Ok(org_id) = Uuid::parse_str(&who.organization_id) else {
                return Ok(LedgerAdjustments { rows: Vec::new() });
            };

            let mut sql = String::from(
                "SELECT \
                    r.id AS request_id, \
                    r.object_id, \
                    r.status AS request_status, \
                    r.proposed_payload, \
                    r.requested_by, \
                    r.requested_at, \
                    d.approver_id AS decided_by, \
                    d.at AS decided_at, \
                    d.reason AS decision_reason, \
                    p.id AS posting_id, \
                    p.posted_at, \
                    v.valuations \
                 FROM approval_requests r \
                 LEFT JOIN approval_decisions d ON d.request_id = r.id \
                 LEFT JOIN ledger_postings p \
                    ON p.organization_id = r.organization_id \
                   AND p.source_type = 'ADJUSTMENT' \
                   AND p.source_id = r.object_id \
                   AND p.source_key = 'post' \
                 LEFT JOIN LATERAL ( \
                    SELECT json_agg( \
                        json_build_object( \
                            'currency', trim(e.currency), \
                            'quantity', e.quantity_signed::text, \
                            'valuation_thb', e.valuation_thb::text \
                        ) ORDER BY e.id \
                    ) AS valuations \
                    FROM ledger_entries e \
                    JOIN cash_accounts a ON a.id = e.account_id AND a.kind = 'SAFE' \
                    WHERE e.posting_id = p.id \
                 ) v ON true \
                 WHERE r.type = 'LEDGER_ADJUSTMENT' \
                   AND r.organization_id = $1 \
                   AND r.proposed_payload->>'branch' = $2",
            );

            if !status_filter.is_empty() {
                sql.push_str(
                    " AND (CASE WHEN p.id IS NOT NULL THEN 'POSTED' ELSE r.status END) = $3",
                );
            }
            sql.push_str(" ORDER BY r.requested_at DESC, r.id DESC");

            let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(org_id)
                .bind(&branch_id);

            if !status_filter.is_empty() {
                query = query.bind(&status_filter);
            }

            let db_rows = query
                .fetch_all(&pool)
                .await
                .map_err(pos_error::AppError::Database)?;

            let mut rows = Vec::with_capacity(db_rows.len());
            for r in db_rows {
                let req_id: Uuid = r.get("request_id");
                let obj_id: String = r.get("object_id");
                let req_status: String = r.get("request_status");
                let payload: Value = r.get("proposed_payload");
                let requested_by: String = r.get("requested_by");
                let req_at: DateTime<Utc> = r.get("requested_at");
                let decided_by: Option<String> = r.get("decided_by");
                let decided_at: Option<DateTime<Utc>> = r.get("decided_at");
                let decision_reason: Option<String> = r.get("decision_reason");
                let posting_id: Option<Uuid> = r.get("posting_id");
                let posted_at: Option<DateTime<Utc>> = r.get("posted_at");
                let raw_valuations: Option<Value> = r.get("valuations");

                let is_posted = posting_id.is_some();
                let status = if is_posted {
                    "POSTED".to_string()
                } else {
                    req_status
                };

                let kind = payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();

                let place = payload
                    .get("place")
                    .and_then(Value::as_str)
                    .map(String::from);

                let lines: Vec<LedgerAdjustmentLine> = payload
                    .get("lines")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();

                let reason = payload
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();

                let evidence: LedgerAdjustmentEvidence = payload
                    .get("evidence")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();

                let exception_id = payload
                    .get("exceptionId")
                    .or_else(|| payload.get("exception_id"))
                    .and_then(Value::as_str)
                    .map(String::from);

                let valuations = if is_posted {
                    match raw_valuations {
                        Some(Value::Array(items)) => items
                            .into_iter()
                            .map(|item| {
                                let currency = item
                                    .get("currency")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                let qty_str =
                                    item.get("quantity").and_then(Value::as_str).unwrap_or("0");
                                let val_str = item
                                    .get("valuation_thb")
                                    .and_then(Value::as_str)
                                    .unwrap_or("0");
                                let qty = Decimal::from_str(qty_str)
                                    .unwrap_or_default()
                                    .normalize()
                                    .to_string();
                                let val = Decimal::from_str(val_str).unwrap_or_default().abs();
                                LedgerAdjustmentPostedLine {
                                    currency,
                                    quantity: qty,
                                    valuation_thb: format!("{:.2}", val),
                                }
                            })
                            .collect(),
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                };

                rows.push(LedgerAdjustmentRow {
                    id: obj_id,
                    approval_request_id: req_id.to_string(),
                    status,
                    kind,
                    place,
                    lines,
                    reason,
                    evidence,
                    exception_id,
                    requested_by,
                    requested_at: req_at.to_rfc3339(),
                    decided_by,
                    decided_at: decided_at.map(|t| t.to_rfc3339()),
                    decision_reason,
                    posting_id: if is_posted {
                        posting_id.map(|id| id.to_string())
                    } else {
                        None
                    },
                    posted_at: if is_posted {
                        posted_at.map(|t| t.to_rfc3339())
                    } else {
                        None
                    },
                    valuations,
                });
            }

            Ok(LedgerAdjustments { rows })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(LedgerAdjustmentsView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    LedgerAdjustmentsFilter::export_all(&cfg)?;
    LedgerAdjustmentRow::export_all(&cfg)?;
    LedgerAdjustments::export_all(&cfg)?;
    Ok(())
}
