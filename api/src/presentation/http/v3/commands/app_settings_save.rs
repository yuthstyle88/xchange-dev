//! `appSettingsSave` (APP-4 #994, design `customer-app.md` §4.4, owner C4): `PUT /api/app-settings`
//! upserts the organisation's planner-draft lifetime. The policy row (`SystemSettings`) admits ADMIN
//! and EXECUTIVE; `AppSettingsService` checks the permission again.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use ts_rs::TS;
use xc_app::application::settings::app_settings_service::AppSettingsService;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::planner::{AppSettingsSaveInput, AppSettingsSaveOutput};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::planner_draft_save::customer_app_error;
use crate::presentation::http::v3::error::V3Error;

pub struct AppSettingsSaveCommand {
    service: AppSettingsService,
}

impl CommandAction for AppSettingsSaveCommand {
    const NAME: &'static str = "appSettingsSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "appSettingsSave");
    type Input = AppSettingsSaveInput;
    type Output = AppSettingsSaveOutput;
    const ROUTE: &'static str = "/api/app-settings";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let saved = self
            .service
            .save_in(tx, who, input.planner_draft_ttl_seconds, chrono::Utc::now())
            .await
            .map_err(customer_app_error)?;
        Ok(AppSettingsSaveOutput {
            planner_draft_ttl_seconds: saved
                .planner_draft_ttl_seconds
                .map_or(0, |ttl| ttl.get()),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(AppSettingsSaveCommand {
            service: AppSettingsService::new(state.db.clone()),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    AppSettingsSaveInput::export_all(&cfg)?;
    AppSettingsSaveOutput::export_all(&cfg)?;
    Ok(())
}
