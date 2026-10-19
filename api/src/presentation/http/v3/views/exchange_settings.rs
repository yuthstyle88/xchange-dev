//! #3401: `GET /api/v3/views/exchange_settings` reads the organisation's quote lifetime and the
//! branch's approval limit (design `quote-transaction.md` §4.2).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::exchange_settings::ExchangeSettingsViewData;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct ExchangeSettingsView {
    pool: PgPool,
}

impl ExchangeSettingsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for ExchangeSettingsView {
    const NAME: &'static str = "exchange_settings";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "exchangeSettingsSave");
    type Data = ExchangeSettingsViewData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let settings = PgSettingsRepository::new(pool.clone());
        let branch = branch.clone();
        async move {
            let mut tx = pool.begin().await?;
            let org: Uuid =
                sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_one(&mut *tx)
                    .await?;
            let quote_ttl_seconds = settings.get_ttl_in(&mut tx, org).await?;
            let approval_limit_thb = settings
                .approval_limit_in(&mut tx, org, &branch)
                .await?
                .map(|m| m.to_string());
            tx.rollback().await?;
            Ok(ExchangeSettingsViewData {
                quote_ttl_seconds,
                approval_limit_thb,
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(ExchangeSettingsView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ExchangeSettingsViewData::export_all(&cfg)?;
    Ok(())
}
