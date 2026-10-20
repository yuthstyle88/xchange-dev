//! `pricingConfigure` (pricing-tiers.md §6, §3, §3.1): one currency's override margins, minimum spread and anchor,
//! saved as a new policy version through `PolicyVersionWriter` (no draft). A frame change: EXECUTIVE only.
//!
//! SKELETON (PT-0): `run` is PT-5's. The name, route, permission (`RatePolicySave`, inherited from `ratePolicySave`)
//! and the input/output types stay.

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

/// One tier's margins as decimal strings (signed, scale 4).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_configure.ts")]
pub struct PricingTierMarginInput {
    pub tier_no: u8,
    pub buy_margin_pct: String,
    pub sell_margin_pct: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_configure.ts")]
pub struct PricingConfigureInput {
    /// The policy version the page was built on; a newer one ⇒ `conflict`.
    pub policy_version_id: String,
    pub currency: String,
    /// Exactly four tiers 1..=4, or `null` to remove the currency's margin override (back to the branch's).
    pub tiers: Option<Vec<PricingTierMarginInput>>,
    /// 0..=5, or `null` for none.
    pub min_spread_pct: Option<String>,
    /// `BOT` · `MARKET_TT` · `MARKET_SR`, or `null` to follow the branch.
    pub anchor_source: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_configure.ts")]
pub struct PricingConfigureOutput {
    pub id: String,
    pub version: i32,
}

pub struct PricingConfigureCommand;

impl CommandAction for PricingConfigureCommand {
    const NAME: &'static str = "pricingConfigure";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingConfigureInput;
    type Output = PricingConfigureOutput;
    const ROUTE: &'static str = "/api/pricing/configure";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        todo!("PT-5: pricingConfigure")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingConfigureCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingTierMarginInput::export_all(&cfg)?;
    PricingConfigureInput::export_all(&cfg)?;
    PricingConfigureOutput::export_all(&cfg)?;
    Ok(())
}
