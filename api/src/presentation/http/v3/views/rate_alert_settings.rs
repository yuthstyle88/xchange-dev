//! RA-2b (#1429): `GET /api/v3/views/rate_alert_settings` reads the organisation's rate alert settings.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::rate_alerts::RateAlertSettingsData;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct RateAlertSettingsView {
    pool: PgPool,
}

impl RateAlertSettingsView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for RateAlertSettingsView {
    const NAME: &'static str = "rate_alert_settings";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateAlertSettings");
    type Data = Option<RateAlertSettingsData>;
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

            let row: Option<(i32, i32, Option<i32>, String)> = sqlx::query_as(
                "SELECT default_cooldown_seconds, max_active_per_customer, default_lifetime_days, \
                 observe_policy \
                 FROM rate_alert_settings \
                 WHERE organization_id = $1",
            )
            .bind(org_id)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?;

            Ok(row.map(
                |(cooldown, max_active, lifetime, policy)| RateAlertSettingsData {
                    default_cooldown_seconds: cooldown,
                    max_active_per_customer: max_active,
                    default_lifetime_days: lifetime,
                    observe_policy: policy,
                },
            ))
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(RateAlertSettingsView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RateAlertSettingsData::export_all(&cfg)?;
    Ok(())
}
