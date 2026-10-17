//! The `handover_receivers` view (CA-8 #624; lane controller's ruling issuecomment-6107853376): the active users of
//! the session branch's organisation who hold `ShiftOperate` at the session branch, the caller excluded, ordered by name and user id. A valid receiver
//! is exactly a person who could later accept (`PersonRules::receiver_verifies_handover`). No clocked-in filter: the
//! server requires none. Authorised like `handoverStart` (`ShiftOperate`, the same row).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::Utc;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor as User;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::application::handover::handover_service::{may_receive, receiver_candidates};
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::handover_receivers::{HandoverReceiverRow, HandoverReceivers};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

struct HandoverReceiversView {
    pool: PgPool,
}

impl BranchView for HandoverReceiversView {
    const NAME: &'static str = "handover_receivers";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "handoverStart");
    type Data = HandoverReceivers;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &User,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        let me = who.user_id.clone();
        async move {
            let now = Utc::now();
            // the session branch's organisation: only its people can receive here (PR 1325 review, issuecomment-6108602501)
            let session_org: Option<String> =
                sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&pool)
                    .await
                    .map_err(AppError::Database)?;
            let Some(session_org) = session_org else {
                return Ok(HandoverReceivers { rows: Vec::new() });
            };
            // the one receiver rule shared with `handoverStart` (#2846, #2879): valid role rows of that organisation
            // reaching this branch, decided by the Authorizer's ShiftOperate at the session branch
            let candidates = receiver_candidates(&pool, &session_org, &branch, None, now).await?;
            let receivers: Vec<HandoverReceiverRow> = candidates
                .into_iter()
                .filter(|c| c.user_id != me && may_receive(c, &session_org, &branch, now))
                .map(|c| HandoverReceiverRow {
                    user_id: c.user_id,
                    name: c.name,
                })
                .collect();
            Ok(HandoverReceivers { rows: receivers })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(HandoverReceiversView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    HandoverReceiverRow::export_all(&cfg)?;
    HandoverReceivers::export_all(&cfg)?;
    Ok(())
}
