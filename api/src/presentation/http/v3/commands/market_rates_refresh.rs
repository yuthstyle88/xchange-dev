//! `marketRatesRefresh` (pricing-tiers.md §15.5 gap 6): the `/pricing` fetch button. Runs `MarketRefresh::tick` over
//! `AppState::market_sources` (TT and Super Rich), then one FOLLOW plan per FOLLOW currency when the caller's branch
//! is MAIN. Gated on `MarketRatesRefresh` (`pricing.market:refresh`). `rateReferenceFetch` stays BoT.
//!
//! SKELETON (XC-V9-PRICING): `run` is PV-g's. Types, route and registration stay.

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
use crate::presentation::dto::v3::pricing_board::PricingFollowResultDto;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "market_rates_refresh.ts")]
pub struct MarketRatesRefreshInput {}

/// One market source's fetch.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "market_rates_refresh.ts")]
pub struct MarketRefreshSourceDto {
    /// `TT` · `SR`.
    pub source: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "market_rates_refresh.ts")]
pub struct MarketRatesRefreshOutput {
    pub sources: Vec<MarketRefreshSourceDto>,
    /// One per FOLLOW currency planned (empty away from MAIN).
    pub follow_results: Vec<PricingFollowResultDto>,
}

pub struct MarketRatesRefreshCommand {
    state: AppState,
}

impl CommandAction for MarketRatesRefreshCommand {
    const NAME: &'static str = "marketRatesRefresh";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "marketRatesRefresh");
    type Input = MarketRatesRefreshInput;
    type Output = MarketRatesRefreshOutput;
    const ROUTE: &'static str = "/api/pricing/market-refresh";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (&self.state, tx, branch, who, input);
        todo!("PV-g: marketRatesRefresh")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(MarketRatesRefreshCommand {
            state: state.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    MarketRatesRefreshInput::export_all(&cfg)?;
    MarketRefreshSourceDto::export_all(&cfg)?;
    MarketRatesRefreshOutput::export_all(&cfg)?;
    Ok(())
}
