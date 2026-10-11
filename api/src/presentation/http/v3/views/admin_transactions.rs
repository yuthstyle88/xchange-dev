//! The `admin_transactions` view (§7.4, §12.1, §12.2).

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
use crate::presentation::dto::v3::admin_transactions::{
    AdminTransactionStatus, AdminTransactionType, AdminTransactionsFilter,
};
use crate::presentation::dto::v3::transaction_row::{
    TransactionRow, TxSqlRow, ROW_COLUMNS, ROW_FROM,
};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct AdminTransactionsPagedView {
    pool: PgPool,
}

impl AdminTransactionsPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' => out.push_str("\\%"),
            '_' => out.push_str("\\_"),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out
}

impl PagedView for AdminTransactionsPagedView {
    const NAME: &'static str = "admin_transactions";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = TransactionRow;
    type Filter = AdminTransactionsFilter;

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
            builder.push(" AND t.mode = 'LIVE'");

            match filter.status.as_deref() {
                Some("CANCELLED") => {
                    builder.push(" AND t.status = 'CANCELLED'");
                }
                Some("PENDING") => {
                    builder.push(" AND t.status IN ('DRAFT', 'PENDING_APPROVAL', 'CONFIRMED')");
                }
                Some("COMPLETED") | None => {
                    builder.push(" AND t.status = 'COMPLETED'");
                }
                Some(other) => {
                    builder.push(" AND t.status = ");
                    builder.push_bind(other);
                }
            }

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
                let pattern = format!("%{}%", escape_like(&q.to_lowercase()));
                builder.push(" AND ((t.document_ref IS NOT NULL AND lower(t.document_ref) LIKE ");
                builder.push_bind(pattern.clone());
                builder.push(" ESCAPE '\\') OR lower(t.customer_name) LIKE ");
                builder.push_bind(pattern.clone());
                builder.push(" ESCAPE '\\' OR lower(l.currency::text) LIKE ");
                builder.push_bind(pattern);
                builder.push(" ESCAPE '\\')");
            }

            if let Some(ref r#type) = filter.r#type
                && (r#type == "BUY" || r#type == "SELL")
            {
                builder.push(" AND l.shop_side = ");
                builder.push_bind(r#type);
            }

            if let Some(ref staff) = filter.staff
                && !staff.is_empty()
            {
                builder.push(" AND t.staff_id = ");
                builder.push_bind(staff);
            }

            if let Some(ref staff_q) = filter.staff_q
                && !staff_q.is_empty()
            {
                let pattern = format!("%{}%", escape_like(&staff_q.to_lowercase()));
                builder.push(" AND lower(t.staff_id) LIKE ");
                builder.push_bind(pattern);
                builder.push(" ESCAPE '\\'");
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
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(AdminTransactionsPagedView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminTransactionType::export_all(&cfg)?;
    AdminTransactionStatus::export_all(&cfg)?;
    AdminTransactionsFilter::export_all(&cfg)?;
    TransactionRow::export_all(&cfg)?;
    Ok(())
}
