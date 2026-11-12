//! `GET /api/v3/views/pricing_rules` (owner-set-values.md §2; xchange#6555): the organisation's current pricing rules
//! and every saved version, newest first. A field that is `null` is "not set"; an organisation with no save answers
//! `current` with every field `null` and an empty `history`. Needs `SettingsRead`, through `exchangeSettingsRead`.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::domain::policy::pricing_rules::PricingRules;
use xc_rate::infrastructure::pricing::pricing_rules_store::{
    PricingRulesStore, PricingRulesVersion,
};

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::http::v3::setter_names::SetterNames;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

/// One band of `rateDecimals`: prices from `fromPrice` up take `decimals` places.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_rules.ts")]
pub struct RateDecimalsBandDto {
    pub from_price: String,
    pub decimals: u32,
}

/// An inclusive % range, decimal strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_rules.ts")]
pub struct PctRangeDto {
    pub min: String,
    pub max: String,
}

/// The pricing board's advice thresholds, decimal strings, set together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_rules.ts")]
pub struct AdviceThresholdsDto {
    pub far_pct: String,
    pub spread_ratio: String,
    pub spread_gap_pt: String,
    pub market_moved_pct: String,
}

/// The rules, each `null` when not set. Decimals are strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_rules.ts")]
pub struct PricingRulesDto {
    pub far_from_anchor_limit_pct: Option<String>,
    pub large_move_pct: Option<String>,
    pub rate_decimals: Option<Vec<RateDecimalsBandDto>>,
    pub thb_cash_step: Option<String>,
    pub branch_bp_cap: Option<i32>,
    pub better_pct_range: Option<PctRangeDto>,
    pub min_spread_pct_range: Option<PctRangeDto>,
    pub market_stale_after_minutes: Option<i32>,
    pub advice: Option<AdviceThresholdsDto>,
}

impl From<&PricingRules> for PricingRulesDto {
    fn from(r: &PricingRules) -> Self {
        let s = |d: rust_decimal::Decimal| d.normalize().to_string();
        let range = |p: &xc_rate::domain::policy::pricing_rules::PctRange| PctRangeDto {
            min: s(p.min),
            max: s(p.max),
        };
        Self {
            far_from_anchor_limit_pct: r.far_from_anchor_limit_pct.map(s),
            large_move_pct: r.large_move_pct.map(s),
            rate_decimals: r.rate_decimals.as_ref().map(|d| {
                d.bands
                    .iter()
                    .map(|&(from, decimals)| RateDecimalsBandDto {
                        from_price: s(from),
                        decimals,
                    })
                    .collect()
            }),
            thb_cash_step: r.thb_cash_step.map(s),
            branch_bp_cap: r.branch_bp_cap,
            better_pct_range: r.better_pct_range.as_ref().map(range),
            min_spread_pct_range: r.min_spread_pct_range.as_ref().map(range),
            // `validate` keeps it within i32, the column's type.
            market_stale_after_minutes: r.market_stale_after.map(|d| d.num_minutes() as i32),
            advice: r.advice.as_ref().map(|a| AdviceThresholdsDto {
                far_pct: s(a.far_pct),
                spread_ratio: s(a.spread_ratio),
                spread_gap_pt: s(a.spread_gap_pt),
                market_moved_pct: s(a.market_moved_pct),
            }),
        }
    }
}

/// One saved version; `version`, `setBy` and `createdAt` are `null` on the "not set" answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_rules.ts")]
pub struct PricingRulesVersionDto {
    pub version: Option<String>,
    /// The saver's user id.
    pub set_by: Option<String>,
    /// The saver's name, `null` when the user has none.
    pub set_by_name: Option<String>,
    /// RFC 3339.
    pub created_at: Option<String>,
    pub rules: PricingRulesDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_rules.ts")]
pub struct PricingRulesViewData {
    pub current: PricingRulesVersionDto,
    /// Every saved version, newest first (the current one included).
    pub history: Vec<PricingRulesVersionDto>,
}

/// `versions` as DTOs, with the savers' names.
pub(crate) async fn version_dtos(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    versions: &[PricingRulesVersion],
) -> AppResult<Vec<PricingRulesVersionDto>> {
    let ids: Vec<String> = versions.iter().filter_map(|v| v.set_by.clone()).collect();
    let names: HashMap<String, String> = SetterNames::of_in(tx, &ids).await?;
    Ok(versions
        .iter()
        .map(|v| PricingRulesVersionDto {
            version: v.version.clone(),
            set_by: v.set_by.clone(),
            set_by_name: v.set_by.as_ref().and_then(|id| names.get(id).cloned()),
            created_at: v.created_at.map(|t| t.to_rfc3339()),
            rules: PricingRulesDto::from(&v.rules),
        })
        .collect())
}

pub struct PricingRulesView {
    pool: PgPool,
}

impl PricingRulesView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl BranchView for PricingRulesView {
    const NAME: &'static str = "pricing_rules";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "exchangeSettingsRead");
    type Data = PricingRulesViewData;
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
            let mut tx = pool.begin().await?;
            let org: Uuid =
                sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_one(&mut *tx)
                    .await?;
            let store_err = |e: xc_rate::domain::draft::rate_engine_error::RateEngineError| {
                AppError::Internal(e.to_string())
            };
            let current = PricingRulesStore::current_in(&mut tx, org)
                .await
                .map_err(store_err)?;
            let history = PricingRulesStore::history_in(&mut tx, org)
                .await
                .map_err(store_err)?;
            let current = version_dtos(&mut tx, std::slice::from_ref(&current))
                .await?
                .remove(0);
            let history = version_dtos(&mut tx, &history).await?;
            tx.rollback().await?;
            Ok(PricingRulesViewData { current, history })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(PricingRulesView::new(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingRulesViewData::export_all(&cfg)?;
    Ok(())
}
