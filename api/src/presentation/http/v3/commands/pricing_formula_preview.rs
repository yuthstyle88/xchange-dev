//! `pricingFormulaPreview` (pricing-formula.md §4, B17): the §4 check on a formula the owner is typing, answered as a
//! table and written nowhere. The organisation formula editor and the branch override block both call it.
//!
//! SKELETON (pricing-formula): types and signature fixed here; `run` is one function ticket.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_kernel::money::RatePercent;
use xc_rate::application::formula::spread_check::{
    BranchCandidate, SpreadCandidate, SpreadCheck, SpreadFailure, SpreadMode,
};
use xc_rate::domain::formula::price_formula::PriceFormula;
use xc_rate::domain::formula::text::FormulaText;
use xc_rate::domain::policy::rate_policy::RatePolicy;
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;
use xc_rate::infrastructure::policy::policy_tiers_store::PolicyTiersStore;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input;
use crate::presentation::http::v3::rate_policy_save::may_change_the_frame;

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
        let scope = input::one_of("scope", &input.scope, &["ORGANISATION", "BRANCH"])?;
        let now = Utc::now();
        let allowed = if scope == "ORGANISATION" {
            who.roles.iter().any(|r| {
                r.role == Role::Executive
                    && r.branch.is_none()
                    && r.valid_from <= now
                    && r.valid_to.is_none_or(|to| now < to)
            })
        } else {
            may_change_the_frame(who, branch, now)
        };
        if !allowed {
            return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
        }

        let parse_one = |field: &str, text: &str| {
            FormulaText::parse(text).map_err(|e| FormulaParseErrorDto {
                field: field.to_owned(),
                code: e.code().to_owned(),
                message: e.to_string(),
            })
        };
        let (buy, sell) = match parse_one("buyFormula", &input.buy_formula)
            .and_then(|b| parse_one("sellFormula", &input.sell_formula).map(|s| (b, s)))
        {
            Ok(pair) => pair,
            Err(err) => {
                return Ok(PricingFormulaPreviewOutput {
                    parse_error: Some(err),
                    rows: Vec::new(),
                    not_checked: Vec::new(),
                    ok: false,
                });
            }
        };
        let formula = PriceFormula::new(buy, sell);

        let candidate = if scope == "ORGANISATION" {
            let organization_id: Uuid = match Uuid::parse_str(&who.organization_id) {
                Ok(id) => id,
                Err(_) => sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(|_| server_error())?,
            };
            SpreadCandidate::Organisation {
                organization_id,
                formula,
            }
        } else {
            let Some((ver_id, buy_dec, sell_dec, guards)): Option<(
                Uuid,
                Decimal,
                Decimal,
                Value,
            )> = sqlx::query_as(
                "SELECT id, buy_adjust_pct, sell_adjust_pct, guards \
                 FROM rate_policy_versions \
                 WHERE branch_id = $1 AND valid_to IS NULL",
            )
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| server_error())?
            else {
                return Ok(PricingFormulaPreviewOutput {
                    parse_error: None,
                    rows: Vec::new(),
                    not_checked: Vec::new(),
                    ok: true,
                });
            };
            let buy_pct = RatePercent::parse(&format!("{buy_dec:.4}")).map_err(|_| server_error())?;
            let sell_pct =
                RatePercent::parse(&format!("{sell_dec:.4}")).map_err(|_| server_error())?;
            let policy = RatePolicy::new(ver_id.to_string(), buy_pct, sell_pct, &guards)
                .map_err(|_| server_error())?;
            let tiers = PolicyTiersStore::for_version(tx, &ver_id)
                .await
                .map_err(|_| server_error())?;
            let overrides = CurrencyTierOverrideStore::for_version(tx, &ver_id)
                .await
                .map_err(|_| server_error())?;
            SpreadCandidate::Branch {
                branch: branch.clone(),
                version: BranchCandidate {
                    formula: Some(formula),
                    policy,
                    tiers,
                    overrides,
                },
            }
        };

        let report = SpreadCheck::run(tx, &candidate)
            .await
            .map_err(|_| server_error())?;
        let ok = report.is_ok();
        let rows = report
            .priced
            .iter()
            .map(|r| {
                let failure = report
                    .failures
                    .iter()
                    .find(|f| {
                        f.branch == r.branch
                            && f.currency == r.currency
                            && f.tier == r.tier
                            && f.mode == r.mode
                    })
                    .map(SpreadFailure::describe);
                FormulaCheckRowDto {
                    branch: r.branch.as_str().to_owned(),
                    currency: r.currency.as_str().to_owned(),
                    tier: r.tier,
                    mode: match r.mode {
                        SpreadMode::Auto => "AUTO".to_owned(),
                        SpreadMode::Follow => "FOLLOW".to_owned(),
                    },
                    buy: r.buy.map(|d| d.normalize().to_string()),
                    sell: r.sell.map(|d| d.normalize().to_string()),
                    failure,
                }
            })
            .collect();
        let not_checked = report
            .not_checked
            .into_iter()
            .map(|(b, c)| FormulaNotCheckedDto {
                branch: b.as_str().to_owned(),
                currency: c.as_str().to_owned(),
            })
            .collect();

        Ok(PricingFormulaPreviewOutput {
            parse_error: None,
            rows,
            not_checked,
            ok,
        })
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
