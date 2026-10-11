//! The `admin_customer_history` view (§7.4, §12.1).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_customer_history::AdminCustomerHistoryFilter;
use crate::presentation::dto::v3::transaction_row::{
    TransactionRow, TxSqlRow, ROW_COLUMNS, ROW_FROM,
};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct AdminCustomerHistoryPagedView {
    pool: PgPool,
}

impl AdminCustomerHistoryPagedView {
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

impl PagedView for AdminCustomerHistoryPagedView {
    const NAME: &'static str = "admin_customer_history";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = TransactionRow;
    type Filter = AdminCustomerHistoryFilter;

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
            let customer_id = match &filter.customer {
                Some(c) if !c.trim().is_empty() => c.trim(),
                _ => return Ok(vec![]),
            };

            let before_cursor: Option<(DateTime<Utc>, i64)> = if let Some(before_id) = before {
                let cursor = if let Ok(u) = uuid::Uuid::parse_str(&before_id) {
                    sqlx::query_as::<_, (DateTime<Utc>, i64)>(
                        "SELECT recorded_at, seq FROM transactions WHERE branch_id = $1 AND customer_id = $2 AND mode = 'LIVE' AND (document_ref = $3 OR id = $4)",
                    )
                    .bind(branch.as_str())
                    .bind(customer_id)
                    .bind(&before_id)
                    .bind(u)
                    .fetch_optional(&pool)
                    .await
                    .map_err(AppError::Database)?
                } else {
                    sqlx::query_as::<_, (DateTime<Utc>, i64)>(
                        "SELECT recorded_at, seq FROM transactions WHERE branch_id = $1 AND customer_id = $2 AND mode = 'LIVE' AND document_ref = $3",
                    )
                    .bind(branch.as_str())
                    .bind(customer_id)
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

            let mut builder = sqlx::QueryBuilder::new("SELECT ");
            builder.push(ROW_COLUMNS);
            builder.push(" FROM ");
            builder.push(ROW_FROM);
            builder.push(" WHERE t.branch_id = ");
            builder.push_bind(branch.as_str());
            builder.push(" AND t.mode = 'LIVE' AND t.status = 'COMPLETED' AND t.customer_id = ");
            builder.push_bind(customer_id);

            if let Some((before_recorded_at, before_seq)) = before_cursor {
                builder.push(" AND (t.recorded_at < ");
                builder.push_bind(before_recorded_at);
                builder.push(" OR (t.recorded_at = ");
                builder.push_bind(before_recorded_at);
                builder.push(" AND t.seq < ");
                builder.push_bind(before_seq);
                builder.push("))");
            }

            if let Some(ref q) = filter.q
                && !q.is_empty()
            {
                let pattern = format!("%{}%", escape_like(&q.to_lowercase()));
                builder.push(" AND (lower(t.document_ref) LIKE ");
                builder.push_bind(pattern.clone());
                builder.push(" ESCAPE '\\' OR lower(l.currency::text) LIKE ");
                builder.push_bind(pattern.clone());
                builder.push(" ESCAPE '\\' OR t.value_date::text LIKE ");
                builder.push_bind(pattern);
                builder.push(" ESCAPE '\\')");
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
        Arc::new(AdminCustomerHistoryPagedView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::admin_customer_history::export_types(dir)
}
