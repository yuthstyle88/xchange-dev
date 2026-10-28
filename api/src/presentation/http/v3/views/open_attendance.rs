//! Staff corrections §8 note 4 (SC-A4): the `open_attendance` view. Other employees' clock-ins at the branch that
//! are still open and began on a day before today in the branch timezone: what a manager may close with
//! `attendanceCloseFor`. Newest clock-in first.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::open_attendance::OpenAttendanceRow;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

pub struct OpenAttendanceView {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
struct SqlRow {
    id: String,
    employee_id: String,
    name: String,
    clock_in_ms: i64,
}

impl PagedView for OpenAttendanceView {
    const NAME: &'static str = "open_attendance";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "openAttendance");
    type Row = OpenAttendanceRow;
    type Filter = NoFilter;

    fn page(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        let caller = who.user_id.clone();
        async move {
            let rows: Vec<SqlRow> = sqlx::query_as(
                "SELECT a.id, a.employee_id, COALESCE(u.name, '') AS name, \
                        (a.data->>'clockIn')::bigint AS clock_in_ms \
                 FROM attendance a \
                 JOIN branches b ON b.id = a.branch_id \
                 LEFT JOIN users u ON u.id::text = a.employee_id \
                 WHERE a.branch_id = $1 \
                   AND a.employee_id <> $2 \
                   AND a.data->>'clockOut' IS NULL \
                   AND (to_timestamp((a.data->>'clockIn')::bigint / 1000.0) AT TIME ZONE b.timezone)::date \
                       < (now() AT TIME ZONE b.timezone)::date \
                   AND ($3::text IS NULL OR a.position < \
                        (SELECT p.position FROM attendance p WHERE p.branch_id = $1 AND p.id = $3)) \
                 ORDER BY a.position DESC \
                 LIMIT $4",
            )
            .bind(&branch)
            .bind(&caller)
            .bind(before.as_deref())
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            Ok(rows
                .into_iter()
                .map(|r| OpenAttendanceRow {
                    id: r.id,
                    employee_id: r.employee_id,
                    name: r.name,
                    clock_in: DateTime::<Utc>::from_timestamp_millis(r.clock_in_ms)
                        .unwrap_or_default()
                        .to_rfc3339_opts(SecondsFormat::Millis, true),
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(OpenAttendanceView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::open_attendance::export_types(dir)
}
