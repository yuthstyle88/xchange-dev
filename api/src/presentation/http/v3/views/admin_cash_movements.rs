//! The `admin_cash_movements` view (§7, §12.6).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::admin_cash_movements::AdminCashMovementRow;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct AdminCashMovementsPagedView {
    pool: PgPool,
}

impl AdminCashMovementsPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct MovementSqlRow {
    id: String,
    shift_id: String,
    at: DateTime<Utc>,
    by: String,
    code: String,
    direction: String,
    amount: Decimal,
    reason: String,
}

impl PagedView for AdminCashMovementsPagedView {
    const NAME: &'static str = "admin_cash_movements";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Row = AdminCashMovementRow;
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
        let branch_id = branch.as_str().to_string();

        async move {
            let rows: Vec<MovementSqlRow> = if let Some(before_id) = before {
                let before_seq: Option<i64> = sqlx::query_scalar(
                    "SELECT seq FROM stock_movements WHERE id = $1 AND branch_id = $2",
                )
                .bind(&before_id)
                .bind(&branch_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;

                let Some(before_seq) = before_seq else {
                    return Ok(vec![]);
                };

                sqlx::query_as(
                    "SELECT id, shift_id, at, by, trim(code) as code, direction, amount, reason \
                     FROM stock_movements \
                     WHERE branch_id = $1 AND seq < $2 \
                     ORDER BY seq DESC \
                     LIMIT $3",
                )
                .bind(&branch_id)
                .bind(before_seq)
                .bind(limit as i64)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            } else {
                sqlx::query_as(
                    "SELECT id, shift_id, at, by, trim(code) as code, direction, amount, reason \
                     FROM stock_movements \
                     WHERE branch_id = $1 \
                     ORDER BY seq DESC \
                     LIMIT $2",
                )
                .bind(&branch_id)
                .bind(limit as i64)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            };

            Ok(rows
                .into_iter()
                .map(|r| AdminCashMovementRow {
                    id: r.id,
                    created: r.at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    shift_id: r.shift_id,
                    direction: r.direction,
                    currency: r.code,
                    amount: format!("{:.2}", r.amount),
                    reason: r.reason,
                    staff: r.by,
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(cfg, Arc::new(AdminCashMovementsPagedView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AdminCashMovementRow::export_all(&cfg)?;
    Ok(())
}
