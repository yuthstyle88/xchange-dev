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
    kind: String,
    counted_by: String,
    counted_at: DateTime<Utc>,
    verified_by: Option<String>,
    verified_at: Option<DateTime<Utc>>,
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
            let rows: Vec<CountSqlRow> = sqlx::query_as(
                "SELECT c.id::text AS id, c.branch_id, c.shift_id, c.kind, c.counted_by::text AS counted_by, \
                        c.counted_at, c.verified_by::text AS verified_by, c.verified_at, c.note \
                 FROM cash_counts c \
                 WHERE c.branch_id = $1 \
                   AND ($2::text IS NULL OR c.shift_id = $2) \
                   AND ($3::text IS NULL \
                        OR ($3 = 'WAITING' AND c.verified_by IS NULL) \
                        OR ($3 = 'VERIFIED' AND c.verified_by IS NOT NULL)) \
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
                by_count.entry(t.count_id).or_default().push(CashCountTotalRow {
                    currency: t.currency.trim().to_owned(),
                    counted: t.counted.to_string(),
                    expected: t.expected.map(|d| d.to_string()),
                    difference: t.difference.map(|d| d.to_string()),
                });
            }

            Ok(rows
                .into_iter()
                .map(|r| CashCountRow {
                    totals: by_count.remove(&r.id).unwrap_or_default(),
                    id: r.id,
                    branch_id: r.branch_id,
                    shift_id: r.shift_id,
                    kind: r.kind,
                    counted_by: r.counted_by,
                    counted_at: stamp(r.counted_at),
                    verified_by: r.verified_by,
                    verified_at: r.verified_at.map(stamp),
                    note: r.note,
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
