//! `marketRatesRefresh` (pricing-tiers.md §15.5 gap 6): the `/pricing` fetch button. Runs `MarketRefresh::tick` over
//! `AppState::market_sources` (TT and Super Rich), then one FOLLOW plan per FOLLOW currency when the caller's branch
//! is MAIN. Gated on `MarketRatesRefresh` (`pricing.market:refresh`). `rateReferenceFetch` stays BoT.
//!
//! SKELETON (XC-V9-PRICING): `run` is PV-g's. Types, route and registration stay.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::market::market_refresh::MarketRefresh;
use xc_rate::application::pricing::branch_push::BranchPush;
use xc_rate::application::pricing::tier_publish::TierSetPublisher;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::pricing_mode::PricingModeStore;
use xc_rate::infrastructure::pricing::branch_policy_store::PgBranchPolicyStore;
use xc_rate::infrastructure::pricing::pricing_mode_store::PgPricingModeStore;
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

use super::commands::{make_approval_service, map_rate_engine_error};
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::rate_feed::market_pricing_loop::MarketPricingLoop;
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
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let now = Utc::now();
        let refresh = Arc::new(MarketRefresh::new(
            self.state.db.clone(),
            self.state.market_sources.clone(),
        ));
        let report = refresh.tick(now).await;
        let sources = report
            .results
            .into_iter()
            .map(|(source, res)| {
                let source_str = match source {
                    MarketSource::Tt => "TT",
                    MarketSource::SuperRich => "SR",
                    MarketSource::Bot => "BOT",
                };
                MarketRefreshSourceDto {
                    source: source_str.to_string(),
                    ok: res.is_ok(),
                    error: res.err(),
                }
            })
            .collect();

        let mut follow_results = Vec::new();
        if branch.as_str() == "MAIN" {
            let currencies = PgPricingModeStore
                .following_in(tx)
                .await
                .map_err(map_rate_engine_error)?;
            if !currencies.is_empty() {
                let draft_service = Arc::new(RateDraftService::new(
                    self.state.db.clone(),
                    make_approval_service(self.state.db.clone()),
                ));
                let publish_service = Arc::new(PublishService::new(self.state.db.clone()));
                let tier_publisher =
                    Arc::new(TierSetPublisher::new(draft_service, publish_service));
                let branch_push = Arc::new(BranchPush::new(
                    self.state.db.clone(),
                    tier_publisher.clone(),
                    Arc::new(PgTierSetStore),
                    Arc::new(PgBranchPolicyStore),
                ));
                let pricing_loop = MarketPricingLoop::new(
                    self.state.db.clone(),
                    refresh,
                    tier_publisher,
                    branch_push,
                );
                for currency in currencies {
                    let res = pricing_loop
                        .plan_currency(currency, now)
                        .await
                        .map_err(map_rate_engine_error)?;
                    follow_results.push(PricingFollowResultDto {
                        currency: res.currency.as_str().to_string(),
                        status: res.status.as_str().to_string(),
                        reasons: res
                            .reasons
                            .into_iter()
                            .map(|r| r.as_str().to_string())
                            .collect(),
                        planned_at: res.planned_at.to_rfc3339(),
                    });
                }
            }
        }

        Ok(MarketRatesRefreshOutput {
            sources,
            follow_results,
        })
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
