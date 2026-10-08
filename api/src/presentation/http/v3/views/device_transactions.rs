//! The `device_transactions` view (§7.4).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::{Money, NoFilter};
use crate::presentation::dto::v3::device_transactions::DeviceTransactionRow;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct DeviceTransactionsPagedView {
    pool: PgPool,
}

impl DeviceTransactionsPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct TxSqlRow {
    document_ref: String,
    r#type: String,
    amount: String,
    code: String,
}

impl PagedView for DeviceTransactionsPagedView {
    const NAME: &'static str = "device_transactions";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Devices, "snapshot");
    type Row = DeviceTransactionRow;
    type Filter = NoFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        async move {
            let rows: Vec<TxSqlRow> = if let Some(before_id) = before {
                let before_seq: Option<i64> = sqlx::query_scalar(
                    "SELECT seq FROM transactions WHERE branch_id = $1 AND mode = 'LIVE' AND document_ref = $2",
                )
                .bind(branch.as_str())
                .bind(&before_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;

                let Some(before_seq) = before_seq else {
                    return Ok(vec![]);
                };

                sqlx::query_as(
                    "SELECT document_ref, type, amount::text, code::text \
                     FROM transactions \
                     WHERE branch_id = $1 AND mode = 'LIVE' AND seq < $2 \
                     ORDER BY seq DESC \
                     LIMIT $3",
                )
                .bind(branch.as_str())
                .bind(before_seq)
                .bind(limit as i64)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            } else {
                sqlx::query_as(
                    "SELECT document_ref, type, amount::text, code::text \
                     FROM transactions \
                     WHERE branch_id = $1 AND mode = 'LIVE' \
                     ORDER BY seq DESC \
                     LIMIT $2",
                )
                .bind(branch.as_str())
                .bind(limit as i64)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            };

            Ok(rows
                .into_iter()
                .map(|r| DeviceTransactionRow {
                    id: r.document_ref,
                    transaction_type: r.r#type,
                    amount: Money(r.amount),
                    currency: r.code.trim().to_string(),
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(cfg, Arc::new(DeviceTransactionsPagedView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DeviceTransactionRow::export_all(&cfg)?;
    Ok(())
}
