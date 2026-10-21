//! QU-7 (#1266): `GET /api/v3/views/queue_board?branch=` (design `queue-tickets.md` §4.4): the
//! desk's board. Inherits `(Rpc, "queueBoard")`, so `QueueOperate` (TELLER, BRANCH_MANAGER).
//!
//! Its version is the branch's `branch_service_status.queue_version`, which every queue
//! transaction raises (`StatusService::refresh_in`), so a poll with `since` answers
//! `{changed: false}` until a ticket or a counter moves.

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
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::queue_board::{
    QueueBoardCalled, QueueBoardCounter, QueueBoardData, QueueBoardWaiting,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct QueueBoardView {
    pool: PgPool,
}

type WaitingRow = (String, String, String, i32, DateTime<Utc>);
type CalledRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<DateTime<Utc>>,
);

impl BranchView for QueueBoardView {
    const NAME: &'static str = "queue_board";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "queueBoard");
    const VERSIONED: bool = true;
    type Data = QueueBoardData;
    type Filter = NoFilter;

    fn own_version(
        &self,
        branch: &BranchId,
        _who: &Actor,
    ) -> impl Future<Output = AppResult<Option<i64>>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        async move {
            sqlx::query_scalar(
                "SELECT queue_version FROM branch_service_status WHERE branch_id = $1",
            )
            .bind(&branch)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)
        }
    }

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_owned();
        let caller = who.user_id.clone();
        async move {
            // Call order (Q6, `CallOrder`): BOOKING before WALK_IN, then the lowest sequence.
            let waiting: Vec<WaitingRow> = sqlx::query_as(
                "SELECT id::text, display_number, service_type, sequence, issued_at \
                   FROM queue_tickets \
                  WHERE branch_id = $1 AND status = 'WAITING' \
                  ORDER BY CASE service_type WHEN 'BOOKING' THEN 0 ELSE 1 END, sequence, issued_at",
            )
            .bind(&branch)
            .fetch_all(&pool)
            .await?;
            let called: Vec<CalledRow> = sqlx::query_as(
                "SELECT id::text, display_number, service_type, status, counter_teller_id::text, \
                        called_at \
                   FROM queue_tickets \
                  WHERE branch_id = $1 AND status IN ('CALLED', 'SERVING') \
                  ORDER BY called_at, display_number",
            )
            .bind(&branch)
            .fetch_all(&pool)
            .await?;
            let my_counter: Option<String> = sqlx::query_scalar(
                "SELECT status FROM queue_counters WHERE branch_id = $1 AND teller_id::text = $2",
            )
            .bind(&branch)
            .bind(&caller)
            .fetch_optional(&pool)
            .await?;
            Ok(QueueBoardData {
                waiting: waiting
                    .into_iter()
                    .map(|(id, display_number, service_type, sequence, issued_at)| {
                        QueueBoardWaiting {
                            id,
                            display_number,
                            service_type,
                            sequence,
                            issued_at: issued_at.to_rfc3339(),
                        }
                    })
                    .collect(),
                called: called
                    .into_iter()
                    .map(
                        |(id, display_number, service_type, status, teller_id, called_at)| {
                            QueueBoardCalled {
                                id,
                                display_number,
                                service_type,
                                status,
                                teller_id,
                                called_at: called_at.map(|t| t.to_rfc3339()),
                            }
                        },
                    )
                    .collect(),
                my_counter: my_counter.map(|status| QueueBoardCounter { status }),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(QueueBoardView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    QueueBoardData::export_all(&cfg)?;
    Ok(())
}
