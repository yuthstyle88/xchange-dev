//! Ticket CA-1d (`docs/design/xc-counter-app.md` §4, §5.4, §8.4): `POST /api/v3/commands/branch_cash_from_browser_save`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::application::settings::cash_from_browser;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_counters.ts")]
pub struct BranchCashFromBrowserSaveInput {
    pub branch: String,
    #[ts(type = "number")]
    pub expected_version: i64,
    pub value: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_counters.ts")]
pub struct BranchCashFromBrowserSaveAnswer {
    pub value: Option<bool>,
    #[ts(type = "number")]
    pub version: i64,
}

pub struct BranchCashFromBrowserSaveCommand {
    store: BranchSettingStore,
}

impl BranchCashFromBrowserSaveCommand {
    pub fn new(pool: PgPool) -> Self {
        Self {
            store: BranchSettingStore::new(pool),
        }
    }
}

impl CommandAction for BranchCashFromBrowserSaveCommand {
    const NAME: &'static str = "branchCashFromBrowserSave";
    const INHERITS: (EntryPath, &'static str) =
        (EntryPath::OwnerRoute, "branchCashFromBrowserSave");
    type Input = BranchCashFromBrowserSaveInput;
    type Output = BranchCashFromBrowserSaveAnswer;
    const ROUTE: &'static str = "/api/v3/commands/branch_cash_from_browser_save";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let saved = cash_from_browser::save(
            &self.store,
            tx,
            branch,
            input.expected_version,
            input.value,
            &who.user_id,
            Utc::now(),
        )
        .await
        .map_err(|_| server_error())?;

        match saved {
            Ok(stored) => Ok(BranchCashFromBrowserSaveAnswer {
                value: input.value,
                version: stored.version,
            }),
            Err(_) => Err(V3Error::new(
                "staleVersion",
                None,
                "stale version",
                StatusCode::CONFLICT,
            )),
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(BranchCashFromBrowserSaveCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchCashFromBrowserSaveInput::export_all(&cfg)?;
    BranchCashFromBrowserSaveAnswer::export_all(&cfg)?;
    Ok(())
}
