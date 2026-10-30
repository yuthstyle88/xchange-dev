//! `pricingModeSave` (pricing-tiers.md §15.5; `H:` §6): FOLLOW or MANUAL for one currency, at MAIN only. An EXECUTIVE
//! frame change (`may_change_the_frame`), V.9's OWNER. Bumps `pricing_modes.revision`. Another branch ⇒
//! `validationFailed`.
//!
//! SKELETON (XC-V9-PRICING): `run` is PV-f4's. Types, route and registration stay.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::domain::pricing::pricing_mode::{PricingMode, PricingModeStore};
use xc_rate::infrastructure::pricing::pricing_mode_store::PgPricingModeStore;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
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
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "pricingModeSave");
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
        if branch.as_str() != "MAIN" {
            return Err(V3Error::new(
                "validationFailed",
                Some("branch"),
                "pricingModeSave is only allowed at MAIN",
                StatusCode::BAD_REQUEST,
            ));
        }

        let currency = CurrencyCode::parse(input.currency.trim()).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("currency"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;

        let mode = PricingMode::parse(input.mode.trim()).ok_or_else(|| {
            V3Error::new(
                "validationFailed",
                Some("mode"),
                "unknown pricing mode",
                StatusCode::BAD_REQUEST,
            )
        })?;

        let now = Utc::now();
        if !crate::presentation::http::v3::rate_policy_save::may_change_the_frame(who, branch, now)
        {
            return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
        }

        PgPricingModeStore
            .save_in(tx, currency, mode, &who.user_id, now)
            .await
            .map_err(|_| server_error())?;

        Ok(PricingModeSaveOutput { saved: true })
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
