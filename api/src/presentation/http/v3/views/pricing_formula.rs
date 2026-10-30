//! `GET /api/v3/views/pricing_formula?branch=` (pricing-formula.md §6): the organisation's current formula, the
//! built-in default, the inputs a formula may name, and which formula each branch of the organisation follows.
//!
//! SKELETON (pricing-formula): types and signature fixed here; `load` is one function ticket.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde::Serialize;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::domain::formula::input::FormulaInput;
use xc_rate::domain::formula::price_formula::{DEFAULT_BUY, DEFAULT_SELL};

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

#[derive(sqlx::FromRow)]
struct OrgFormulaRow {
    id: Uuid,
    version: i32,
    buy_formula: String,
    sell_formula: String,
    valid_from: DateTime<Utc>,
    approved_by: String,
    reason: String,
}

impl BranchView for PricingFormulaView {
    const NAME: &'static str = "pricing_formula";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "pricing_formula");
    const VERSIONED: bool = false;
    type Data = PricingFormulaData;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.clone();
        async move {
            let mut tx = pool.begin().await.map_err(AppError::Database)?;

            let org_id: Option<Uuid> =
                sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(AppError::Database)?;

            let org_id = org_id.ok_or_else(|| AppError::NotFound {
                entity: "branch",
                id: branch.to_string(),
            })?;

            let org_row: Option<OrgFormulaRow> = sqlx::query_as(
                "SELECT id, version, buy_formula, sell_formula, valid_from, approved_by, reason \
                 FROM pricing_formula_versions \
                 WHERE organization_id = $1 AND valid_to IS NULL",
            )
            .bind(org_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(AppError::Database)?;

            let has_org_formula = org_row.is_some();
            let organisation = org_row.map(|row| OrganisationFormulaDto {
                id: row.id.to_string(),
                version: row.version,
                buy_formula: row.buy_formula,
                sell_formula: row.sell_formula,
                valid_from: row.valid_from.to_rfc3339(),
                approved_by: row.approved_by,
                reason: row.reason,
            });

            let branch_rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT b.id, rpv.buy_formula, rpv.sell_formula \
                 FROM branches b \
                 LEFT JOIN LATERAL ( \
                     SELECT buy_formula, sell_formula \
                     FROM rate_policy_versions \
                     WHERE branch_id = b.id AND valid_to IS NULL \
                     ORDER BY version DESC \
                     LIMIT 1 \
                 ) rpv ON true \
                 WHERE b.organization_id = $1 \
                 ORDER BY b.id ASC",
            )
            .bind(org_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(AppError::Database)?;

            tx.commit().await.map_err(AppError::Database)?;

            let branches = branch_rows
                .into_iter()
                .map(|(branch, buy, sell)| {
                    let (source, own) = match (buy, sell) {
                        (Some(buy_formula), Some(sell_formula)) => (
                            "BRANCH".to_string(),
                            Some(FormulaTextsDto {
                                buy_formula,
                                sell_formula,
                            }),
                        ),
                        _ if has_org_formula => ("ORGANISATION".to_string(), None),
                        _ => ("DEFAULT".to_string(), None),
                    };
                    BranchFormulaDto {
                        branch,
                        source,
                        own,
                    }
                })
                .collect();

            let default = FormulaTextsDto {
                buy_formula: DEFAULT_BUY.to_string(),
                sell_formula: DEFAULT_SELL.to_string(),
            };

            let inputs = FormulaInput::ALL
                .into_iter()
                .map(|i| FormulaInputDto {
                    name: i.name().to_string(),
                })
                .collect();

            Ok(PricingFormulaData {
                organisation,
                default,
                inputs,
                branches,
            })
        }
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
