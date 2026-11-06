//! #6273: `POST /api/v3/commands/kiosk_screen_save` — the owner saves a branch's kiosk screen values.
//!
//! EXECUTIVE only (`kioskScreenSave` = `SystemSettings`, seed v10). Versioned and audited like the other
//! `branch_settings` saves: stored under `kiosk_screen::SETTING` through `BranchSettingStore::write_in`, with one
//! audit row per save. The bounds and defaults are `application::kiosk_screen`'s; the refresh period's upper bound
//! is `kiosk_screen::MAX_REFRESH_SECONDS`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::kiosk_screen::{KioskScreenData, KioskScreenSaveInput};
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

pub struct KioskScreenSaveCommand {
    #[allow(dead_code)]
    pool: PgPool,
}

impl KioskScreenSaveCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for KioskScreenSaveCommand {
    const NAME: &'static str = "kioskScreenSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "kioskScreenSave");
    type Input = KioskScreenSaveInput;
    type Output = KioskScreenData;
    const ROUTE: &'static str = "/api/v3/commands/kiosk_screen_save";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Err(V3Error::from(PolicyError::SignIn))
    }

    async fn run_as(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _caller: &Caller,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        todo!("#6273: validate, check the version, write through BranchSettingStore, audit")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(KioskScreenSaveCommand::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    KioskScreenSaveInput::export_all(&cfg)?;
    Ok(())
}
