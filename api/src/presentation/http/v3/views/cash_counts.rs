//! The `cash_counts` view (CA-8 #624; design `cash-shift-transfer.md` §4.7): the branch's counts, newest first, paged
//! by `limit` and `before` (the parent contract), filtered by `shift` and by `status` = `WAITING` (`verified_by IS
//! NULL`) or `VERIFIED` (Q-A ruling on #624). Area row `cashCounts` (G2).

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::cash::{CashCountRow, CashCountTotalRow, CashCountsFilter};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct CashCountsView {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
struct CountSqlRow {
    id: String,
    branch_id: String,
    shift_id: Option<String>,
    shift_opened_at: Option<DateTime<Utc>>,
    kind: String,
    counted_by: String,
    counted_by_name: Option<String>,
    counted_at: DateTime<Utc>,
    verified_by: Option<String>,
    verified_by_name: Option<String>,
    verified_at: Option<DateTime<Utc>>,
    rejected_by: Option<String>,
    rejected_at: Option<DateTime<Utc>>,
    reject_reason: Option<String>,
    approval_request_id: Option<String>,
    superseded: bool,
    note: Option<String>,
}

#[derive(sqlx::FromRow)]
struct TotalSqlRow {
    count_id: String,
    currency: String,
    counted: Decimal,
    expected: Option<Decimal>,
    difference: Option<Decimal>,
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn non_blank(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.trim().is_empty())
}

impl PagedView for CashCountsView {
    const NAME: &'static str = "cash_counts";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "cashCounts");
    type Row = CashCountRow;
    type Filter = CashCountsFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        let filter = filter.clone();
        async move {
            // `superseded` uses the same newest-count predicate as `is_newest_count`
            // (`api/crates/xc-cash/src/infrastructure/repository.rs:212-245`).
            let rows: Vec<CountSqlRow> = sqlx::query_as(
                "SELECT c.id::text AS id, c.branch_id, c.shift_id, s.opened_at AS shift_opened_at, \
                        c.kind, c.counted_by::text AS counted_by, cu.name AS counted_by_name, \
                        c.counted_at, c.verified_by::text AS verified_by, vu.name AS verified_by_name, \
                        c.verified_at, c.rejected_by::text AS rejected_by, c.rejected_at, \
                        c.reject_reason, ar.id::text AS approval_request_id, sup.superseded, c.note \
                 FROM cash_counts c \
                 LEFT JOIN shifts s ON s.id = c.shift_id \
                 LEFT JOIN users cu ON cu.id = c.counted_by \
                 LEFT JOIN users vu ON vu.id = c.verified_by \
                 LEFT JOIN approval_requests ar \
                        ON ar.type = 'COUNT_DIFFERENCE' AND ar.status = 'PENDING' AND ar.object_id = c.id::text \
                 CROSS JOIN LATERAL ( \
                     SELECT EXISTS ( \
                         SELECT 1 FROM cash_counts n \
                         WHERE n.kind = c.kind \
                           AND n.id <> c.id \
                           AND (n.created_at > c.created_at OR n.counted_at > c.counted_at) \
                           AND ((c.shift_id IS NOT NULL AND n.shift_id = c.shift_id) \
                                OR (c.shift_id IS NULL AND n.shift_id IS NULL \
                                    AND n.branch_id = c.branch_id AND n.counted_by = c.counted_by)) \
                     ) AS superseded \
                 ) sup \
                 WHERE c.branch_id = $1 \
                   AND ($2::text IS NULL OR c.shift_id = $2) \
                   AND ($3::text IS NULL \
                        OR ($3 = 'WAITING' \
                            AND c.verified_by IS NULL AND c.rejected_by IS NULL \
                            AND ar.id IS NULL AND NOT sup.superseded) \
                        OR ($3 = 'AWAITING_FINANCE' \
                            AND c.verified_by IS NULL AND c.rejected_by IS NULL \
                            AND ar.id IS NOT NULL) \
                        OR ($3 = 'VERIFIED' AND c.verified_by IS NOT NULL) \
                        OR ($3 = 'REJECTED' AND c.rejected_by IS NOT NULL)) \
                   AND ($4::text IS NULL OR (c.created_at, c.id) < \
                        (SELECT b.created_at, b.id FROM cash_counts b WHERE b.id::text = $4 AND b.branch_id = $1)) \
                 ORDER BY c.created_at DESC, c.id DESC \
                 LIMIT $5",
            )
            .bind(&branch)
            .bind(filter.shift.as_deref())
            .bind(filter.status.as_deref())
            .bind(before.as_deref())
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
            let totals: Vec<TotalSqlRow> = sqlx::query_as(
                "SELECT count_id::text AS count_id, currency::text AS currency, counted, expected, difference \
                 FROM cash_count_totals WHERE count_id::text = ANY($1) ORDER BY currency",
            )
            .bind(&ids)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            let mut by_count: BTreeMap<String, Vec<CashCountTotalRow>> = BTreeMap::new();
            for t in totals {
                let difference = t.difference.or_else(|| t.expected.map(|e| t.counted - e));
                by_count
                    .entry(t.count_id)
                    .or_default()
                    .push(CashCountTotalRow {
                        currency: t.currency.trim().to_owned(),
                        counted: t.counted.to_string(),
                        expected: t.expected.map(|d| d.to_string()),
                        difference: difference.map(|d| d.to_string()),
                    });
            }

            Ok(rows
                .into_iter()
                .map(|r| {
                    let status = if r.verified_by.is_some() {
                        "VERIFIED"
                    } else if r.rejected_by.is_some() {
                        "REJECTED"
                    } else if r.approval_request_id.is_some() {
                        "AWAITING_FINANCE"
                    } else {
                        "WAITING"
                    }
                    .to_owned();
                    CashCountRow {
                        totals: by_count.remove(&r.id).unwrap_or_default(),
                        id: r.id,
                        branch_id: r.branch_id,
                        shift_id: r.shift_id,
                        shift_opened_at: r.shift_opened_at.map(stamp),
                        kind: r.kind,
                        status,
                        counted_by: r.counted_by,
                        counted_by_name: non_blank(r.counted_by_name),
                        counted_at: stamp(r.counted_at),
                        verified_by: r.verified_by,
                        verified_by_name: non_blank(r.verified_by_name),
                        verified_at: r.verified_at.map(stamp),
                        rejected_by: r.rejected_by,
                        rejected_at: r.rejected_at.map(stamp),
                        reject_reason: r.reject_reason,
                        approval_request_id: r.approval_request_id,
                        superseded: r.superseded,
                        note: r.note,
                    }
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(CashCountsView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CashCountsFilter::export_all(&cfg)?;
    CashCountTotalRow::export_all(&cfg)?;
    CashCountRow::export_all(&cfg)?;
    Ok(())
}
