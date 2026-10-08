//! The `device_events` view (§7.3).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::devices::device_repository::AMOUNT_KEYS;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::device_events::DeviceEventRow;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct DeviceEventsPagedView {
    pool: PgPool,
}

impl DeviceEventsPagedView {
    fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct EventSqlRow {
    seq: i64,
    kind: String,
    reference: String,
    at: DateTime<Utc>,
    actor: String,
    detail: String,
}

impl PagedView for DeviceEventsPagedView {
    const NAME: &'static str = "device_events";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Devices, "snapshot");
    type Row = DeviceEventRow;
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
            let rows: Vec<EventSqlRow> = if let Some(before_id) = before {
                let Ok(before_seq) = before_id.parse::<i64>() else {
                    return Ok(vec![]);
                };

                sqlx::query_as(
                    "SELECT seq, kind, ref AS reference, at, actor, \
                     (detail || COALESCE((SELECT jsonb_object_agg(a.key, detail->>a.key) \
                                          FROM unnest($4::text[]) AS a(key) \
                                          WHERE jsonb_typeof(detail->a.key) = 'number'), '{}'::jsonb))::text AS detail \
                     FROM device_events \
                     WHERE branch_id = $1 AND seq < $2 \
                     ORDER BY seq DESC \
                     LIMIT $3",
                )
                .bind(branch.as_str())
                .bind(before_seq)
                .bind(limit as i64)
                .bind(AMOUNT_KEYS.as_slice())
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            } else {
                sqlx::query_as(
                    "SELECT seq, kind, ref AS reference, at, actor, \
                     (detail || COALESCE((SELECT jsonb_object_agg(a.key, detail->>a.key) \
                                          FROM unnest($3::text[]) AS a(key) \
                                          WHERE jsonb_typeof(detail->a.key) = 'number'), '{}'::jsonb))::text AS detail \
                     FROM device_events \
                     WHERE branch_id = $1 \
                     ORDER BY seq DESC \
                     LIMIT $2",
                )
                .bind(branch.as_str())
                .bind(limit as i64)
                .bind(AMOUNT_KEYS.as_slice())
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?
            };

            Ok(rows
                .into_iter()
                .map(|r| DeviceEventRow {
                    seq: r.seq.to_string(),
                    kind: r.kind,
                    reference: r.reference,
                    at: r.at.to_rfc3339_opts(SecondsFormat::Millis, true),
                    actor: r.actor,
                    detail: r.detail,
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(cfg, Arc::new(DeviceEventsPagedView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DeviceEventRow::export_all(&cfg)?;
    Ok(())
}
