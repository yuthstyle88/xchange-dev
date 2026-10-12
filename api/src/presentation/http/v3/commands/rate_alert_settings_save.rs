//! RA-2b (#1429): `PUT /api/rate-alert-settings` saves the organisation's rate alert settings.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_alert::domain::settings::SaveAlertSettings;
use xc_alert::domain::source_policy::ObservePolicy;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use super::rate_alert_create::{make_rate_alert_service, map_alert_error};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::rate_alerts::{
    RateAlertSettingsData, SaveRateAlertSettingsInput,
};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, role_not_allowed_here};
use crate::presentation::http::v3::error::V3Error;

pub struct RateAlertSettingsSaveCommand {
    pool: PgPool,
}

impl RateAlertSettingsSaveCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for RateAlertSettingsSaveCommand {
    const NAME: &'static str = "rateAlertSettingsSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateAlertSettingsSave");
    type Input = SaveRateAlertSettingsInput;
    type Output = RateAlertSettingsData;
    const ROUTE: &'static str = "/api/rate-alert-settings";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let org_id = Uuid::parse_str(&who.organization_id).map_err(|_| role_not_allowed_here())?;
        let updated_by = Uuid::parse_str(&who.user_id).map_err(|_| role_not_allowed_here())?;

        let observe_policy = ObservePolicy::parse(&input.observe_policy).ok_or_else(|| {
            V3Error::new(
                "validationFailed",
                Some("observePolicy"),
                "invalid observe policy",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let save_cmd = SaveAlertSettings {
            default_cooldown_seconds: input.default_cooldown_seconds,
            max_active_per_customer: input.max_active_per_customer,
            default_lifetime_days: input.default_lifetime_days,
            observe_policy,
        };

        let service = make_rate_alert_service(self.pool.clone());
        let settings = service
            .save_settings(tx, org_id, &save_cmd, updated_by, chrono::Utc::now())
            .await
            .map_err(map_alert_error)?;

        Ok(RateAlertSettingsData {
            default_cooldown_seconds: settings.default_cooldown_seconds,
            max_active_per_customer: settings.max_active_per_customer,
            default_lifetime_days: settings.default_lifetime_days,
            observe_policy: settings.observe_policy.as_str().to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(RateAlertSettingsSaveCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    SaveRateAlertSettingsInput::export_all(&cfg)?;
    Ok(())
}
