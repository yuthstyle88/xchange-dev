//! The `device_jobs` view (§7.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::device_jobs::DeviceJobRow;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct DeviceJobsPagedView {
    pool: PgPool,
}

impl DeviceJobsPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct JobSqlRow {
    id: String,
    kind: String,
    reference: String,
    status: String,
    actor: String,
    created_at: DateTime<Utc>,
}

impl PagedView for DeviceJobsPagedView {
    const NAME: &'static str = "device_jobs";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Devices, "snapshot");
    type Row = DeviceJobRow;
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
            let rows: Vec<JobSqlRow> = if let Some(before_id) = before {
                let before_created: Option<DateTime<Utc>> = sqlx::query_scalar(
                    "SELECT created_at FROM device_jobs WHERE branch_id = $1 AND id = $2",
                )
                .bind(branch.as_str())
                .bind(&before_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;

                let Some(before_created) = before_created else {
                    return Ok(vec![]);
                };

                sqlx::query_as(
                    "SELECT id, kind, ref AS reference, status, actor, created_at \
                     FROM device_jobs \
                     WHERE branch_id = $1 AND (created_at < $2 OR (created_at = $2 AND id < $3)) \
                     ORDER BY created_at DESC, id DESC \
                     LIMIT $4",
                )
                .bind(branch.as_str())
                .bind(before_created)
                .bind(&before_id)
                .bind(limit as i64)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            } else {
                sqlx::query_as(
                    "SELECT id, kind, ref AS reference, status, actor, created_at \
                     FROM device_jobs \
                     WHERE branch_id = $1 \
                     ORDER BY created_at DESC, id DESC \
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
                .map(|r| DeviceJobRow {
                    id: r.id,
                    kind: r.kind,
                    reference: r.reference,
                    status: r.status,
                    actor: r.actor,
                    created: r.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(cfg, Arc::new(DeviceJobsPagedView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DeviceJobRow::export_all(&cfg)?;
    Ok(())
}
