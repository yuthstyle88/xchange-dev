//! The `handovers_pending` view (CA-8 #624; design `cash-shift-transfer.md` §4.7, SD B7): the hand-overs in the
//! branch waiting for the caller to accept or reject them, newest first, paged by `limit` and `before`. Area row
//! `handoversPending` (G2).

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
use crate::presentation::dto::v3::cash::HandoverPendingRow;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

struct HandoversPendingView {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
struct HandoverSqlRow {
    id: String,
    from_shift_id: String,
    count_id: String,
    receiver_id: String,
    status: String,
    created_at: DateTime<Utc>,
}

impl PagedView for HandoversPendingView {
    const NAME: &'static str = "handovers_pending";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoversPending");
    type Row = HandoverPendingRow;
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
        let me = who.user_id.clone();
        async move {
            let rows: Vec<HandoverSqlRow> = sqlx::query_as(
                "SELECT h.id::text AS id, h.from_shift_id, h.count_id::text AS count_id, \
                        h.receiver_id::text AS receiver_id, h.status, h.created_at \
                 FROM handovers h \
                 WHERE h.branch_id = $1 AND h.receiver_id::text = $2 AND h.status = 'PENDING' \
                   AND ($3::text IS NULL OR (h.created_at, h.id) < \
                        (SELECT b.created_at, b.id FROM handovers b WHERE b.id::text = $3)) \
                 ORDER BY h.created_at DESC, h.id DESC \
                 LIMIT $4",
            )
            .bind(&branch)
            .bind(&me)
            .bind(before.as_deref())
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            Ok(rows
                .into_iter()
                .map(|r| HandoverPendingRow {
                    id: r.id,
                    from_shift_id: r.from_shift_id,
                    count_id: r.count_id,
                    receiver_id: r.receiver_id,
                    status: r.status,
                    created_at: r.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
                })
                .collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(HandoversPendingView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    HandoverPendingRow::export_all(&cfg)?;
    Ok(())
}
