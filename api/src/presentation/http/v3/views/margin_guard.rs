//! MK-4 (#3402, marketing-campaigns.md §4.5): `GET /api/v3/views/margin_guard` reads the organisation's margin guard.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::margin_guard::MarginGuardData;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct MarginGuardView {
    pool: PgPool,
}

impl MarginGuardView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

type MarginGuardRow = (Option<String>, Option<String>, Option<DateTime<Utc>>);

impl BranchView for MarginGuardView {
    const NAME: &'static str = "margin_guard";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "marginGuardSet");
    type Data = MarginGuardData;
    type Filter = NoFilter;

    fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let org_id_str = who.organization_id.clone();
        async move {
            let org_id = Uuid::parse_str(&org_id_str)
                .map_err(|e| AppError::Validation(format!("invalid org_id: {e}")))?;

            let row: Option<MarginGuardRow> = sqlx::query_as(
                "SELECT max_spread_share::text, set_by::text, set_at \
                   FROM margin_guards \
                  WHERE organization_id = $1",
            )
            .bind(org_id)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;

            Ok(match row {
                Some((max_spread_share, set_by, set_at)) => MarginGuardData {
                    max_spread_share,
                    set_by,
                    set_at: set_at.map(|t| t.to_rfc3339()),
                },
                None => MarginGuardData::default(),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(MarginGuardView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MarginGuardData::export_all(&cfg)?;
    Ok(())
}
