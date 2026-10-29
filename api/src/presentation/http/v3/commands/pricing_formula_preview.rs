//! `pricingFormulaPreview` (pricing-formula.md §4, B17): the §4 check on a formula the owner is typing, answered as a
//! table and written nowhere. The organisation formula editor and the branch override block both call it.
//!
//! SKELETON (pricing-formula): types and signature fixed here; `run` is one function ticket.

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
#[ts(export_to = "pricing_formula_preview.ts")]
pub struct PricingFormulaPreviewInput {
    pub buy_formula: String,
    pub sell_formula: String,
    /// `ORGANISATION` = as the organisation formula (every branch with no own formula); `BRANCH` = as the request
    /// branch's own formula, with its current percentages, tiers and overrides.
    pub scope: String,
}

/// One priced row (`SpreadRow`), with its failure if any.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula_preview.ts")]
pub struct FormulaCheckRowDto {
    pub branch: String,
    pub currency: String,
    /// `null` = the single-price line.
    pub tier: Option<u8>,
    /// `AUTO` · `FOLLOW`.
    pub mode: String,
    /// Decimal strings, normalised; `null` when the side has no price.
    pub buy: Option<String>,
    pub sell: Option<String>,
    /// `null` = passes; else `SpreadFailure::describe()`.
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula_preview.ts")]
pub struct FormulaNotCheckedDto {
    pub branch: String,
    pub currency: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula_preview.ts")]
pub struct PricingFormulaPreviewOutput {
    /// `null` when both texts parse; else `{ field, code, message }` of the first that does not.
    pub parse_error: Option<FormulaParseErrorDto>,
    pub rows: Vec<FormulaCheckRowDto>,
    pub not_checked: Vec<FormulaNotCheckedDto>,
    /// True when no row fails and both texts parse: the save would pass the check.
    pub ok: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula_preview.ts")]
pub struct FormulaParseErrorDto {
    /// `buyFormula` · `sellFormula`.
    pub field: String,
    /// `FormulaError::code()`.
    pub code: String,
    pub message: String,
}

pub struct PricingFormulaPreviewCommand;

impl CommandAction for PricingFormulaPreviewCommand {
    const NAME: &'static str = "pricingFormulaPreview";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingFormulaPreviewInput;
    type Output = PricingFormulaPreviewOutput;
    const ROUTE: &'static str = "/api/pricing/formula/preview";
    const METHOD: Method = Method::POST;

    /// EXECUTIVE only (`may_change_the_frame`, §5; `ORGANISATION` needs an organisation-wide EXECUTIVE role) else
    /// `403 forbidden`. Parses both texts (a parse error is answered in `parse_error`, not as an error status), runs
    /// `SpreadCheck::run` and answers every row; writes nothing.
    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input);
        todo!("pricing-formula wave 3: pricingFormulaPreview")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingFormulaPreviewCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingFormulaPreviewInput::export_all(&cfg)?;
    FormulaCheckRowDto::export_all(&cfg)?;
    FormulaNotCheckedDto::export_all(&cfg)?;
    FormulaParseErrorDto::export_all(&cfg)?;
    PricingFormulaPreviewOutput::export_all(&cfg)?;
    Ok(())
}
