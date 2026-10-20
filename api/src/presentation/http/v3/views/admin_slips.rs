//! The `admin_slips` view (§7.4, §12.1, §12.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_slips::{AdminSlipStatus, AdminSlipsFilter};
use crate::presentation::dto::v3::transaction_row::{
    TransactionRow, TxSqlRow, ROW_COLUMNS, ROW_FROM,
};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint, push_literal_contains};

struct AdminSlipsPagedView {
    pool: PgPool,
}

impl AdminSlipsPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PagedView for AdminSlipsPagedView {
    const NAME: &'static str = "admin_slips";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = TransactionRow;
    type Filter = AdminSlipsFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let filter = filter.clone();

        async move {
            // In v3, only completed exchanges are stored; cancelled or pending filter matches nothing.
            if let Some(ref status) = filter.status
                && (status == "CANCELLED" || status == "PENDING")
            {
                return Ok(vec![]);
            }

            let before_cursor: Option<(DateTime<Utc>, i64)> = if let Some(before_id) = before {
                let cursor: Option<(DateTime<Utc>, i64)> = if let Ok(uuid) = uuid::Uuid::parse_str(&before_id) {
                    sqlx::query_as(
                        "SELECT recorded_at, seq FROM transactions WHERE branch_id = $1 AND mode = 'LIVE' AND (document_ref = $2 OR id = $3)",
                    )
                    .bind(branch.as_str())
                    .bind(&before_id)
                    .bind(uuid)
                    .fetch_optional(&pool)
                    .await
                    .map_err(AppError::Database)?
                } else {
                    sqlx::query_as(
                        "SELECT recorded_at, seq FROM transactions WHERE branch_id = $1 AND mode = 'LIVE' AND document_ref = $2",
                    )
                    .bind(branch.as_str())
                    .bind(&before_id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(AppError::Database)?
                };

                let Some(cursor) = cursor else {
                    return Ok(vec![]);
                };
                Some(cursor)
            } else {
                None
            };

            let mut builder = sqlx::QueryBuilder::new(format!(
                "SELECT {ROW_COLUMNS} FROM {ROW_FROM} WHERE t.branch_id = "
            ));
            builder.push_bind(branch.as_str());
            builder.push(" AND t.mode = 'LIVE' AND t.status = 'COMPLETED' AND p.slip_file_key IS NOT NULL");

            if let Some((before_recorded_at, before_seq)) = before_cursor {
                builder.push(" AND (t.recorded_at < ");
                builder.push_bind(before_recorded_at);
                builder.push(" OR (t.recorded_at = ");
                builder.push_bind(before_recorded_at);
                builder.push(" AND t.seq < ");
                builder.push_bind(before_seq);
                builder.push("))");
            }

            if let Some(ref from) = filter.from
                && !from.is_empty()
                && let Ok(d) = chrono::NaiveDate::parse_from_str(from, "%Y-%m-%d")
            {
                builder.push(" AND t.value_date >= ");
                builder.push_bind(d);
            }

            if let Some(ref to) = filter.to
                && !to.is_empty()
                && let Ok(d) = chrono::NaiveDate::parse_from_str(to, "%Y-%m-%d")
            {
                builder.push(" AND t.value_date <= ");
                builder.push_bind(d);
            }

            if let Some(ref q) = filter.q
                && !q.is_empty()
            {
                push_literal_contains(
                    &mut builder,
                    &[
                        "t.document_ref",
                        "t.customer_name",
                        "t.customer_id",
                        "l.currency::text",
                        "p.reference",
                    ],
                    q,
                );
            }

            builder.push(" ORDER BY t.recorded_at DESC, t.seq DESC LIMIT ");
            builder.push_bind(limit as i64);

            let query = builder.build_query_as::<TxSqlRow>();
            let sql_rows = query.fetch_all(&pool).await.map_err(AppError::Database)?;

            let mut rows = Vec::with_capacity(sql_rows.len());
            for r in sql_rows {
                rows.push(TransactionRow::from_sql_row(r)?);
            }

            Ok(rows)
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(cfg, Arc::new(AdminSlipsPagedView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminSlipStatus::export_all(&cfg)?;
    AdminSlipsFilter::export_all(&cfg)?;
    TransactionRow::export_all(&cfg)?;
    Ok(())
}
