//! `GET /api/v3/views/pricing_formula?branch=` (pricing-formula.md §6): the organisation's current formula, the
//! built-in default, the inputs a formula may name, and which formula each branch of the organisation follows.
//!
//! SKELETON (pricing-formula): types and signature fixed here; `load` is one function ticket.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::AppResult;
use serde::Serialize;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula.ts")]
pub struct FormulaTextsDto {
    pub buy_formula: String,
    pub sell_formula: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula.ts")]
pub struct OrganisationFormulaDto {
    pub id: String,
    pub version: i32,
    pub buy_formula: String,
    pub sell_formula: String,
    /// RFC 3339.
    pub valid_from: String,
    pub approved_by: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula.ts")]
pub struct BranchFormulaDto {
    pub branch: String,
    /// `BRANCH` · `ORGANISATION` · `DEFAULT` (what `FormulaResolver::resolve` answers for its current version).
    pub source: String,
    /// The branch's own texts when `source` is `BRANCH`, else `null`.
    pub own: Option<FormulaTextsDto>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula.ts")]
pub struct FormulaInputDto {
    /// `FormulaInput::name()`.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_formula.ts")]
pub struct PricingFormulaData {
    pub organisation: Option<OrganisationFormulaDto>,
    /// `DEFAULT_BUY` / `DEFAULT_SELL`.
    pub default: FormulaTextsDto,
    /// `FormulaInput::ALL`, in that order.
    pub inputs: Vec<FormulaInputDto>,
    /// Every branch of the request branch's organisation, by branch id.
    pub branches: Vec<BranchFormulaDto>,
}

pub struct PricingFormulaView {
    pool: PgPool,
}

impl PricingFormulaView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for PricingFormulaView {
    const NAME: &'static str = "pricing_formula";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    const VERSIONED: bool = false;
    type Data = PricingFormulaData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let _ = (&self.pool, branch);
        async move { todo!("pricing-formula wave 3: pricing_formula view") }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(PricingFormulaView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    FormulaTextsDto::export_all(&cfg)?;
    OrganisationFormulaDto::export_all(&cfg)?;
    BranchFormulaDto::export_all(&cfg)?;
    FormulaInputDto::export_all(&cfg)?;
    PricingFormulaData::export_all(&cfg)?;
    Ok(())
}
