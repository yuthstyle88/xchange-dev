//! Rate drafts view (§4.7).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use serde_json::Value;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::rate_drafts::{RateDraftItemDto, RateDraftsData};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct RateDraftsBranchView {
    pool: PgPool,
}

impl RateDraftsBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for RateDraftsBranchView {
    const NAME: &'static str = "rate_drafts";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    const VERSIONED: bool = true;
    type Data = RateDraftsData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();

        async move {
            let rows = sqlx::query(
                "SELECT id, branch_id, status, version, created_by, created_at, updated_at, evaluation \
                 FROM rate_drafts WHERE branch_id = $1 ORDER BY created_at DESC",
            )
            .bind(&branch_id)
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let mut drafts = Vec::new();
            for r in rows {
                let id: Uuid = r.get(0);
                let b_id: String = r.get(1);
                let status: String = r.get(2);
                let version: i32 = r.get(3);
                let created_by: String = r.get(4);
                let created_at: DateTime<Utc> = r.get(5);
                let updated_at: DateTime<Utc> = r.get(6);
                let evaluation: Option<Value> = r.get(7);

                drafts.push(RateDraftItemDto {
                    id: id.to_string(),
                    branch_id: b_id,
                    status,
                    version,
                    created_by,
                    created_at: created_at.to_rfc3339(),
                    updated_at: updated_at.to_rfc3339(),
                    evaluation,
                });
            }

            Ok(RateDraftsData { drafts })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(RateDraftsBranchView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RateDraftItemDto::export_all(&cfg)?;
    RateDraftsData::export_all(&cfg)?;
    Ok(())
}
