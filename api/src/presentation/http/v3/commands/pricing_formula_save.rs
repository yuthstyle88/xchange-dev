//! `pricingFormulaSave` (pricing-formula.md §3, §4, §5): the organisation's next formula version, written only after
//! the §4 check passes.
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
use crate::presentation::http::v3::pricing_formula_preview::FormulaNotCheckedDto;
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula_save.ts")]
pub struct PricingFormulaSaveInput {
    pub buy_formula: String,
    pub sell_formula: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula_save.ts")]
pub struct PricingFormulaSaveOutput {
    /// The new `pricing_formula_versions` row.
    pub id: String,
    pub version: i32,
    /// Currencies with no current reference line: saved without being checked (§4 step 4, B9).
    pub not_checked: Vec<FormulaNotCheckedDto>,
}

pub struct PricingFormulaSaveCommand;

impl CommandAction for PricingFormulaSaveCommand {
    const NAME: &'static str = "pricingFormulaSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingFormulaSaveInput;
    type Output = PricingFormulaSaveOutput;
    const ROUTE: &'static str = "/api/pricing/formula";
    const METHOD: Method = Method::POST;

    /// In order: an organisation-wide EXECUTIVE (§5, F-2) else `403 forbidden`; blank `reason` ⇒ `400` field
    /// `reason`; each text through `FormulaText::parse` ⇒ `400 validationFailed`, field `buyFormula` / `sellFormula`,
    /// message `FormulaError`'s; `SpreadCheck::run(Organisation)` with failures ⇒ `400 validationFailed`, field
    /// `buyFormula`, message = one `SpreadFailure::describe()` per line, nothing written; else close the current
    /// `pricing_formula_versions` row (`valid_to = now()`), insert version max + 1 with `source_version` = the
    /// closed one, and answer it with `not_checked`.
    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (tx, branch, who, input);
        todo!("pricing-formula wave 3: pricingFormulaSave")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingFormulaSaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingFormulaSaveInput::export_all(&cfg)?;
    PricingFormulaSaveOutput::export_all(&cfg)?;
    Ok(())
}
