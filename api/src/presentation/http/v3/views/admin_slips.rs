//! The `admin_slips` view (§7.4, §12.1, §12.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_slips::{AdminSlipStatus, AdminSlipsFilter};
use crate::presentation::dto::v3::transaction_row::{
    TransactionRecord, TransactionRow, TxSqlRow,
};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct AdminSlipsPagedView {
    pool: PgPool,
}

impl AdminSlipsPagedView {
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
                let cursor: Option<(DateTime<Utc>, i64)> = sqlx::query_as(
                    "SELECT recorded_at, seq FROM transactions WHERE branch_id = $1 AND mode = 'LIVE' AND document_ref = $2",
                )
                .bind(branch.as_str())
                .bind(&before_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;

                let Some(cursor) = cursor else {
                    return Ok(vec![]);
                };
                Some(cursor)
            } else {
                None
            };

            let mut builder = sqlx::QueryBuilder::new(
                "SELECT t.document_ref, t.value_date, t.recorded_at, t.type, t.code::text AS code, \
                 t.amount::text AS amount, t.rate::text AS rate, t.reference_rate::text AS reference_rate, \
                 t.unit_size::text AS unit_size, \
                 t.fee::text AS fee, t.bank_fee::text AS bank_fee, t.method, t.payment_ref, \
                 t.customer_id, t.customer_name, t.staff_id, t.notes, t.shift_id, \
                 f.file_key AS slip_key \
                 FROM transactions t \
                 JOIN transaction_files f ON f.transaction_id = t.id AND f.kind = 'SLIP' \
                 WHERE t.branch_id = ",
            );
            builder.push_bind(branch.as_str());
            builder.push(" AND t.mode = 'LIVE'");

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
                let pattern = format!("%{}%", escape_like(q));
                builder.push(
                    " AND (lower(normalize(t.document_ref, NFKC)) LIKE lower(normalize(",
                );
                builder.push_bind(pattern.clone());
                builder.push(", NFKC)) ESCAPE '\\' \
                     OR lower(normalize(t.customer_name, NFKC)) LIKE lower(normalize(");
                builder.push_bind(pattern.clone());
                builder.push(", NFKC)) ESCAPE '\\' \
                     OR lower(normalize(t.customer_id, NFKC)) LIKE lower(normalize(");
                builder.push_bind(pattern.clone());
                builder.push(", NFKC)) ESCAPE '\\' \
                     OR lower(normalize(t.code::text, NFKC)) LIKE lower(normalize(");
                builder.push_bind(pattern.clone());
                builder.push(", NFKC)) ESCAPE '\\' \
                     OR lower(normalize(t.payment_ref, NFKC)) LIKE lower(normalize(");
                builder.push_bind(pattern);
                builder.push(", NFKC)) ESCAPE '\\')");
            }

            builder.push(" ORDER BY t.recorded_at DESC, t.seq DESC LIMIT ");
            builder.push_bind(limit as i64);

            let query = builder.build_query_as::<TxSqlRow>();
            let sql_rows = query.fetch_all(&pool).await.map_err(AppError::Database)?;

            let mut rows = Vec::with_capacity(sql_rows.len());
            for r in sql_rows {
                let record = TransactionRecord::try_from(r)?;
                rows.push(TransactionRow::from_record(&record));
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
