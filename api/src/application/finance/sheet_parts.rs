//! FN-2 #1069: F01 finance sheet parts (counts, transfers, integrity, reversals pending).

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_cash::application::reconcile::{Reconcile, ReconcileScope};
use xc_cash::domain::transfer::cash_transfer::TransferLine;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

use crate::presentation::dto::v3::finance_reconciliation::{
    CountLine, InTransitRow, ShortReceiptRow, TransferStatusCount,
};

use super::reconciliation::{
    BoxFuture, CountRow, IntegrityRow, Part, ReversalPendingRow, SheetPartsPort, TransferSummary,
};

pub struct SheetParts {
    pool: PgPool,
    reconcile: Arc<Reconcile>,
}

impl SheetParts {
    pub fn new(pool: PgPool, reconcile: Arc<Reconcile>) -> Self {
        Self { pool, reconcile }
    }
}

fn money(d: Decimal) -> String {
    let mut d = d;
    d.rescale(2);
    d.to_string()
}

fn money_from_decimal(d: Decimal) -> AppResult<Money> {
    Money::parse(&d.to_string()).map_err(|e| {
        tracing::error!(face_value = %d, error = %e, "invalid stored denomination face_value");
        AppError::Internal(format!("parse money ({d}): {e}"))
    })
}

fn format_minor(minor: i64) -> String {
    Money::from_minor(minor)
        .map(|m| m.to_string())
        .unwrap_or_else(|_| "0.00".to_string())
}

impl SheetPartsPort for SheetParts {
    fn counts<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<CountRow>>>> {
        Box::pin(async move {
            let count_rows = sqlx::query(
                "SELECT c.id, c.kind, c.counted_by, c.verified_by \
                 FROM cash_counts c \
                 JOIN branches b ON b.id = c.branch_id \
                 WHERE c.branch_id = $1 \
                   AND c.verified_at IS NOT NULL \
                   AND (c.counted_at AT TIME ZONE b.timezone)::date = $2 \
                 ORDER BY c.counted_at ASC, c.id ASC",
            )
            .bind(branch.as_str())
            .bind(date)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            if count_rows.is_empty() {
                return Ok(Part::of(Vec::new()));
            }

            let count_ids: Vec<Uuid> = count_rows.iter().map(|r| r.get("id")).collect();

            let total_rows = sqlx::query(
                "SELECT count_id, currency, counted, expected, difference \
                 FROM cash_count_totals \
                 WHERE count_id = ANY($1) \
                 ORDER BY count_id, currency ASC",
            )
            .bind(&count_ids)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let mut totals_by_count: std::collections::HashMap<Uuid, Vec<CountLine>> =
                std::collections::HashMap::new();

            for row in total_rows {
                let count_id: Uuid = row.get("count_id");
                let currency: String = row.get("currency");
                let counted: Decimal = row.get("counted");
                let expected: Option<Decimal> = row.get("expected");
                let difference: Option<Decimal> = row.get("difference");

                totals_by_count
                    .entry(count_id)
                    .or_default()
                    .push(CountLine {
                        currency: currency.trim().to_string(),
                        counted: money(counted),
                        expected: expected.map(money).unwrap_or_else(|| "0.00".to_string()),
                        difference: difference.map(money).unwrap_or_else(|| "0.00".to_string()),
                    });
            }

            let mut result = Vec::with_capacity(count_rows.len());
            for r in count_rows {
                let id: Uuid = r.get("id");
                let kind: String = r.get("kind");
                let counted_by: Uuid = r.get("counted_by");
                let verified_by: Option<Uuid> = r.get("verified_by");
                let lines = totals_by_count.remove(&id).unwrap_or_default();

                result.push(CountRow {
                    count_id: id.to_string(),
                    kind,
                    counted_by: counted_by.to_string(),
                    verified_by: verified_by.map(|u| u.to_string()),
                    lines,
                });
            }

            Ok(Part::of(result))
        })
    }

    fn transfers<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<TransferSummary>>> {
        Box::pin(async move {
            let status_rows = sqlx::query(
                "SELECT t.status, COUNT(*)::bigint AS count \
                 FROM cash_transfers t \
                 JOIN branches b ON b.id = $1 \
                 WHERE (t.from_branch_id = $1 OR t.to_branch_id = $1) \
                   AND ( \
                     (t.created_at AT TIME ZONE b.timezone)::date = $2 \
                     OR (t.dispatched_at AT TIME ZONE b.timezone)::date = $2 \
                     OR (t.received_at AT TIME ZONE b.timezone)::date = $2 \
                   ) \
                 GROUP BY t.status \
                 ORDER BY t.status ASC",
            )
            .bind(branch.as_str())
            .bind(date)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let by_status: Vec<TransferStatusCount> = status_rows
                .into_iter()
                .map(|r| TransferStatusCount {
                    status: r.get("status"),
                    count: r.get("count"),
                })
                .collect();

            // In transit: DISPATCHED, not received, dispatched on or before that business date
            let in_transit_line_rows = sqlx::query(
                "SELECT t.id, tl.currency, tl.denomination_id, tl.qty_sent, tl.qty_received, d.face_value \
                 FROM cash_transfers t \
                 JOIN branches b ON b.id = $1 \
                 JOIN cash_transfer_lines tl ON tl.transfer_id = t.id \
                 JOIN denominations d ON d.id = tl.denomination_id \
                 WHERE (t.from_branch_id = $1 OR t.to_branch_id = $1) \
                   AND t.status = 'DISPATCHED' \
                   AND t.received_at IS NULL \
                   AND (t.dispatched_at AT TIME ZONE b.timezone)::date <= $2 \
                 ORDER BY t.dispatched_at ASC, t.id ASC, tl.currency ASC",
            )
            .bind(branch.as_str())
            .bind(date)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            struct AggInTransit {
                transfer_id: String,
                currency: String,
                total_minor: i64,
            }

            let mut in_transit_map: Vec<AggInTransit> = Vec::new();

            for r in in_transit_line_rows {
                let transfer_id: Uuid = r.get("id");
                let currency_str: String = r.get("currency");
                let currency_trimmed = currency_str.trim().to_string();
                let denomination_id: i64 = r.get("denomination_id");
                let qty_sent: i32 = r.get("qty_sent");
                let qty_received: Option<i32> = r.get("qty_received");
                let face_value_dec: Decimal = r.get("face_value");

                let line = TransferLine {
                    currency: CurrencyCode::parse(&currency_trimmed)
                        .map_err(|e| AppError::Internal(format!("parse currency: {e}")))?,
                    denomination_id,
                    face_value: money_from_decimal(face_value_dec)?,
                    qty_sent,
                    qty_received,
                };

                let t_str = transfer_id.to_string();
                let line_sent = line
                    .sent_minor()
                    .map_err(|e| AppError::Internal(format!("in-transit line total: {e}")))?;
                if let Some(agg) = in_transit_map
                    .iter_mut()
                    .find(|a| a.transfer_id == t_str && a.currency == currency_trimmed)
                {
                    agg.total_minor = agg
                        .total_minor
                        .checked_add(line_sent)
                        .ok_or_else(|| AppError::Internal("in-transit total overflow".into()))?;
                } else {
                    in_transit_map.push(AggInTransit {
                        transfer_id: t_str,
                        currency: currency_trimmed,
                        total_minor: line_sent,
                    });
                }
            }

            let in_transit: Vec<InTransitRow> = in_transit_map
                .into_iter()
                .map(|a| InTransitRow {
                    transfer_id: a.transfer_id,
                    currency: a.currency,
                    quantity: format_minor(a.total_minor),
                })
                .collect();

            // Short receipts: transfers touching the branch on that business date,
            // received with qty_received != qty_sent
            let short_receipt_line_rows = sqlx::query(
                "SELECT t.id, tl.currency, tl.denomination_id, tl.qty_sent, tl.qty_received, d.face_value \
                 FROM cash_transfers t \
                 JOIN branches b ON b.id = $1 \
                 JOIN cash_transfer_lines tl ON tl.transfer_id = t.id \
                 JOIN denominations d ON d.id = tl.denomination_id \
                 WHERE (t.from_branch_id = $1 OR t.to_branch_id = $1) \
                   AND ( \
                     (t.created_at AT TIME ZONE b.timezone)::date = $2 \
                     OR (t.dispatched_at AT TIME ZONE b.timezone)::date = $2 \
                     OR (t.received_at AT TIME ZONE b.timezone)::date = $2 \
                   ) \
                   AND t.status IN ('RECEIVED', 'RECONCILED') \
                 ORDER BY t.received_at ASC, t.id ASC, tl.currency ASC",
            )
            .bind(branch.as_str())
            .bind(date)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            struct AggShortReceipt {
                transfer_id: String,
                currency: String,
                sent_minor: i64,
                received_minor: i64,
                has_mismatch: bool,
            }

            let mut short_receipt_map: Vec<AggShortReceipt> = Vec::new();

            for r in short_receipt_line_rows {
                let transfer_id: Uuid = r.get("id");
                let currency_str: String = r.get("currency");
                let currency_trimmed = currency_str.trim().to_string();
                let denomination_id: i64 = r.get("denomination_id");
                let qty_sent: i32 = r.get("qty_sent");
                let qty_received: Option<i32> = r.get("qty_received");
                let face_value_dec: Decimal = r.get("face_value");

                let line = TransferLine {
                    currency: CurrencyCode::parse(&currency_trimmed)
                        .map_err(|e| AppError::Internal(format!("parse currency: {e}")))?,
                    denomination_id,
                    face_value: money_from_decimal(face_value_dec)?,
                    qty_sent,
                    qty_received,
                };

                let t_str = transfer_id.to_string();
                let line_sent = line
                    .sent_minor()
                    .map_err(|e| AppError::Internal(format!("short-receipt sent total: {e}")))?;
                let line_received = line.received_minor().map_err(|e| {
                    AppError::Internal(format!("short-receipt received total: {e}"))
                })?;
                let mismatch =
                    line.qty_received.is_some() && line.qty_received != Some(line.qty_sent);

                if let Some(agg) = short_receipt_map
                    .iter_mut()
                    .find(|a| a.transfer_id == t_str && a.currency == currency_trimmed)
                {
                    agg.sent_minor = agg.sent_minor.checked_add(line_sent).ok_or_else(|| {
                        AppError::Internal("short-receipt sent sum overflow".into())
                    })?;
                    agg.received_minor =
                        agg.received_minor
                            .checked_add(line_received)
                            .ok_or_else(|| {
                                AppError::Internal("short-receipt received sum overflow".into())
                            })?;
                    if mismatch {
                        agg.has_mismatch = true;
                    }
                } else {
                    short_receipt_map.push(AggShortReceipt {
                        transfer_id: t_str,
                        currency: currency_trimmed,
                        sent_minor: line_sent,
                        received_minor: line_received,
                        has_mismatch: mismatch,
                    });
                }
            }

            let short_receipts: Vec<ShortReceiptRow> = short_receipt_map
                .into_iter()
                .filter(|a| a.has_mismatch || a.sent_minor != a.received_minor)
                .map(|a| ShortReceiptRow {
                    transfer_id: a.transfer_id,
                    currency: a.currency,
                    sent: format_minor(a.sent_minor),
                    received: format_minor(a.received_minor),
                })
                .collect();

            Ok(Part::of(TransferSummary {
                by_status,
                in_transit,
                short_receipts,
            }))
        })
    }

    fn integrity<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<IntegrityRow>>>> {
        Box::pin(async move {
            let org_id: Option<Uuid> =
                sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(AppError::Database)?;

            let org_id = match org_id {
                Some(id) => id,
                None => {
                    return Err(AppError::NotFound {
                        entity: "Branch",
                        id: branch.to_string(),
                    });
                }
            };

            let scope = ReconcileScope {
                organization_id: org_id.to_string(),
                branch: Some(branch.clone()),
                period: Some((date, date)),
            };

            let report = self.reconcile.run(&self.pool, &scope).await?;

            let rows = report
                .checks
                .into_iter()
                .map(|c| IntegrityRow {
                    check: c.id,
                    ok: c.ok,
                    offenders: c.offenders.len() as u32,
                })
                .collect();

            Ok(Part::of(rows))
        })
    }

    fn reversals_pending<'a>(
        &'a self,
        branch: &'a BranchId,
        _date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<ReversalPendingRow>>>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, object_id, requested_by, requested_at \
                 FROM approval_requests \
                 WHERE branch_id = $1 \
                   AND type = 'TRANSACTION_REVERSAL' \
                   AND status = 'PENDING' \
                 ORDER BY requested_at ASC, id ASC",
            )
            .bind(branch.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let result = rows
                .into_iter()
                .map(|r| {
                    let id: Uuid = r.get("id");
                    let object_id: String = r.get("object_id");
                    let requested_by: String = r.get("requested_by");
                    let requested_at: DateTime<Utc> = r.get("requested_at");

                    ReversalPendingRow {
                        request_id: id.to_string(),
                        transaction_id: object_id,
                        requested_by,
                        created_at: requested_at.to_rfc3339(),
                    }
                })
                .collect();

            Ok(Part::of(result))
        })
    }
}
