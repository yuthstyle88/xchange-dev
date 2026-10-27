//! `rateDisplaySave` (pricing-tiers.md §15.8; V9:store `displaySettings`): what the board shows at a branch, or
//! `clear: true` to read MAIN's again. An EXECUTIVE frame change (`may_change_the_frame`).
//!
//! SKELETON (XC-V9-PRICING): `run` is PV-j's. Types, route and registration stay.

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
#[ts(export_to = "rate_display_save.ts")]
pub struct RateDisplaySaveInput {
    /// `true` deletes the branch's row; the other fields are then ignored.
    #[serde(default)]
    #[ts(optional)]
    pub clear: Option<bool>,
    /// 1 to 4 of `1..=4`, no repeats.
    #[serde(default)]
    pub tiers: Vec<u8>,
    /// At most 10 currency codes, shown first in this order.
    #[serde(default)]
    pub featured: Vec<String>,
    #[serde(default)]
    pub highlight: bool,
    #[serde(default)]
    pub all_market: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_display_save.ts")]
pub struct RateDisplaySaveOutput {
    pub saved: bool,
}

pub struct RateDisplaySaveCommand;

impl CommandAction for RateDisplaySaveCommand {
    const NAME: &'static str = "rateDisplaySave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = RateDisplaySaveInput;
    type Output = RateDisplaySaveOutput;
    const ROUTE: &'static str = "/api/pricing/display";
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
        todo!("PV-j: rateDisplaySave")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(RateDisplaySaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RateDisplaySaveInput::export_all(&cfg)?;
    RateDisplaySaveOutput::export_all(&cfg)?;
    Ok(())
}
