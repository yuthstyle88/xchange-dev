//! `pricingFormulaSave` (pricing-formula.md §3, §4, §5): the organisation's next formula version, written only after
//! the §4 check passes.
//!
//! SKELETON (pricing-formula): types and signature fixed here; `run` is one function ticket.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_rate::application::formula::spread_check::{SpreadCandidate, SpreadCheck};
use xc_rate::domain::formula::price_formula::PriceFormula;
use xc_rate::domain::formula::text::FormulaText;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::pricing_formula_preview::FormulaNotCheckedDto;

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
        let now = Utc::now();
        let is_org_exec = who.roles.iter().any(|r| {
            r.role == Role::Executive
                && r.branch.is_none()
                && r.valid_from <= now
                && r.valid_to.is_none_or(|to| now < to)
        });
        if !is_org_exec {
            return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
        }

        if input.reason.trim().is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("reason"),
                "reason cannot be empty",
                StatusCode::BAD_REQUEST,
            ));
        }

        let buy = FormulaText::parse(&input.buy_formula).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("buyFormula"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let sell = FormulaText::parse(&input.sell_formula).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("sellFormula"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let formula = PriceFormula::new(buy, sell);

        let organization_id: Uuid =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|_| server_error())?
                .ok_or_else(server_error)?;

        let report = SpreadCheck::run(
            tx,
            &SpreadCandidate::Organisation {
                organization_id,
                formula: formula.clone(),
            },
        )
        .await
        .map_err(|_| server_error())?;

        if !report.is_ok() {
            let msg = report
                .failures
                .iter()
                .map(|f| f.describe())
                .collect::<Vec<_>>()
                .join("\n");
            return Err(V3Error::new(
                "validationFailed",
                Some("buyFormula"),
                msg,
                StatusCode::BAD_REQUEST,
            ));
        }

        let source_version: Option<i32> = sqlx::query_scalar(
            "SELECT version FROM pricing_formula_versions \
             WHERE organization_id = $1 AND valid_to IS NULL \
             FOR UPDATE",
        )
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let cur_max: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) FROM pricing_formula_versions WHERE organization_id = $1",
        )
        .bind(organization_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| server_error())?;
        let version = cur_max + 1;

        sqlx::query(
            "UPDATE pricing_formula_versions SET valid_to = now() \
             WHERE organization_id = $1 AND valid_to IS NULL",
        )
        .bind(organization_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO pricing_formula_versions \
             (id, organization_id, version, buy_formula, sell_formula, valid_from, \
              approved_by, reason, source_version) \
             VALUES ($1, $2, $3, $4, $5, now(), $6, $7, $8)",
        )
        .bind(id)
        .bind(organization_id)
        .bind(version)
        .bind(formula.buy.as_str())
        .bind(formula.sell.as_str())
        .bind(&who.user_id)
        .bind(&input.reason)
        .bind(source_version)
        .execute(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let not_checked = report
            .not_checked
            .into_iter()
            .map(|(b, c)| FormulaNotCheckedDto {
                branch: b.as_str().to_owned(),
                currency: c.as_str().to_owned(),
            })
            .collect();

        Ok(PricingFormulaSaveOutput {
            id: id.to_string(),
            version,
            not_checked,
        })
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
