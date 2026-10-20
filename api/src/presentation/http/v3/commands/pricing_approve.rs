//! `pricingApprove` (pricing-tiers.md §6, P-3 "Use xchange's flow"): for 1..n currencies, one new policy version with
//! their margins (through `PolicyVersionWriter`), then one rate draft with one AUTO line per currency, submitted.
//! In-bound ⇒ published by policy; out-of-bound ⇒ waits in A06 for EXECUTIVE. Idempotent through the command's key.
//!
//! SKELETON (PT-0): `run` (and any service the command needs in `register`) is PT-6's. The name, route, permission
//! (`RatePolicySave`, inherited; `RatesDraft` is checked in `run`) and the input/output types stay.

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

/// One tier's approved prices (per the currency's unit size), as decimal strings. The server stores the margin that
/// produces each price over the draft path's base (§2, §5.2).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingTierPriceInput {
    pub tier_no: u8,
    pub buy: String,
    pub sell: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingApproveRowInput {
    pub currency: String,
    /// Exactly four tiers 1..=4.
    pub tiers: Vec<PricingTierPriceInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingApproveInput {
    /// The policy version the board was built on; a newer one ⇒ `conflict`, nothing written.
    pub policy_version_id: String,
    /// One or more currencies (bulk approve).
    pub rows: Vec<PricingApproveRowInput>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_approve.ts")]
pub struct PricingApproveOutput {
    pub policy_version_id: String,
    pub version: i32,
    pub draft_id: String,
    /// The draft's state after submit: `PUBLISHED` when in bound, `PENDING_APPROVAL` when it waits in A06.
    pub draft_state: String,
}

pub struct PricingApproveCommand;

impl CommandAction for PricingApproveCommand {
    const NAME: &'static str = "pricingApprove";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingApproveInput;
    type Output = PricingApproveOutput;
    const ROUTE: &'static str = "/api/pricing/approve";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        todo!("PT-6: pricingApprove")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingApproveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingTierPriceInput::export_all(&cfg)?;
    PricingApproveRowInput::export_all(&cfg)?;
    PricingApproveInput::export_all(&cfg)?;
    PricingApproveOutput::export_all(&cfg)?;
    Ok(())
}
