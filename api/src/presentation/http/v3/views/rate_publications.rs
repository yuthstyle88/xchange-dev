//! Rate publications view (§4.7).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{PgPool, Row};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::rate_publications::{
    RatePublicationItemDto, RatePublicationsData,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct RatePublicationsBranchView {
    pool: PgPool,
}

impl RatePublicationsBranchView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for RatePublicationsBranchView {
    const NAME: &'static str = "rate_publications";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    const VERSIONED: bool = true;
    type Data = RatePublicationsData;
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
            let cur_row = sqlx::query(
                "SELECT current_publication_id FROM branches WHERE id = $1",
            )
            .bind(&branch_id)
            .fetch_optional(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let current_pub_id: Option<Uuid> = cur_row.and_then(|r| r.get(0));

            let rows = sqlx::query(
                "SELECT id, branch_id, version, published_at, published_by, approved_request_id, draft_id \
                 FROM rate_publications WHERE branch_id = $1 ORDER BY version DESC",
            )
            .bind(&branch_id)
            .fetch_all(&pool)
            .await
            .map_err(pos_error::AppError::Database)?;

            let mut publications = Vec::new();
            for r in rows {
                let id: Uuid = r.get(0);
                let b_id: String = r.get(1);
                let version: i32 = r.get(2);
                let published_at: DateTime<Utc> = r.get(3);
                let published_by: String = r.get(4);
                let approved_request_id: Uuid = r.get(5);
                let draft_id: Option<Uuid> = r.get(6);

                let state = if Some(id) == current_pub_id {
                    "PUBLISHED"
                } else {
                    "SUPERSEDED"
                };

                publications.push(RatePublicationItemDto {
                    id: id.to_string(),
                    branch_id: b_id,
                    version,
                    published_at: published_at.to_rfc3339(),
                    published_by,
                    approved_request_id: approved_request_id.to_string(),
                    draft_id: draft_id.map(|u| u.to_string()),
                    state: state.to_string(),
                });
            }

            Ok(RatePublicationsData { publications })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(RatePublicationsBranchView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RatePublicationItemDto::export_all(&cfg)?;
    RatePublicationsData::export_all(&cfg)?;
    Ok(())
}
