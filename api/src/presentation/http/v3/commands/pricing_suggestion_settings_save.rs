//! `pricingSuggestionSettingsSave` (pricing-tiers.md §5.2, §6): a branch's `better` % for one currency or for the
//! branch (`"*"`), V.9's `configure` (`src/pricing.mjs:200-211`). A frame change: EXECUTIVE only. No policy version,
//! no draft.
//!
//! SKELETON (XC-PRICING PX-0): `run` is PX-11's. Types, route and registration stay.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_suggestion_settings_save.ts")]
pub struct PricingSuggestionSettingsSaveInput {
    /// `"*"` for the branch default, or a currency code.
    pub currency: String,
    /// Four decimal strings, % better than the market per tier (each in −2..=2, each ≥ the previous), or `null` to
    /// remove the row (back to the branch default, then V.9's).
    pub better_pct: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_suggestion_settings_save.ts")]
pub struct PricingSuggestionSettingsSaveOutput {
    pub saved: bool,
}

pub struct PricingSuggestionSettingsSaveCommand;

impl CommandAction for PricingSuggestionSettingsSaveCommand {
    const NAME: &'static str = "pricingSuggestionSettingsSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingSuggestionSettingsSaveInput;
    type Output = PricingSuggestionSettingsSaveOutput;
    const ROUTE: &'static str = "/api/pricing/suggestion-settings";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input, key);
        todo!("PX-11: pricingSuggestionSettingsSave")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingSuggestionSettingsSaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingSuggestionSettingsSaveInput::export_all(&cfg)?;
    PricingSuggestionSettingsSaveOutput::export_all(&cfg)?;
    Ok(())
}
