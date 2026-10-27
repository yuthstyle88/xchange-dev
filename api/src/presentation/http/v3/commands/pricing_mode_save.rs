//! `pricingModeSave` (pricing-tiers.md §15.5; `H:` §6): FOLLOW or MANUAL for one currency, at MAIN only. An EXECUTIVE
//! frame change (`may_change_the_frame`), V.9's OWNER. Bumps `pricing_modes.revision`. Another branch ⇒
//! `validationFailed`.
//!
//! SKELETON (XC-V9-PRICING): `run` is PV-f's. Types, route and registration stay.

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
#[ts(export_to = "pricing_mode_save.ts")]
pub struct PricingModeSaveInput {
    pub currency: String,
    /// `"FOLLOW"` · `"MANUAL"`.
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_mode_save.ts")]
pub struct PricingModeSaveOutput {
    pub saved: bool,
}

pub struct PricingModeSaveCommand;

impl CommandAction for PricingModeSaveCommand {
    const NAME: &'static str = "pricingModeSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingModeSaveInput;
    type Output = PricingModeSaveOutput;
    const ROUTE: &'static str = "/api/pricing/mode";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input);
        todo!("PV-f: pricingModeSave")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingModeSaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingModeSaveInput::export_all(&cfg)?;
    PricingModeSaveOutput::export_all(&cfg)?;
    Ok(())
}
