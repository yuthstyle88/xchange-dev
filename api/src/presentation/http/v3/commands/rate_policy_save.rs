//! Rate policy save command (§4.7, §6).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_policy_save.ts")]
pub struct RatePolicySaveInput {
    pub buy_adjust_pct: String,
    pub sell_adjust_pct: String,
    #[ts(type = "any")]
    pub guards: Value,
    pub max_reference_age_hours: Option<i32>,
    pub on_stale: Option<String>,
    pub stale_limit_thb: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_policy_save.ts")]
pub struct RatePolicySaveOutput {
    pub id: String,
    pub version: i32,
}

#[derive(Debug, sqlx::FromRow)]
struct CurrentPolicyRow {
    version: i32,
    guards: Value,
    max_reference_age_hours: Option<i32>,
    on_stale: Option<String>,
    stale_limit_thb: Option<Decimal>,
}

pub struct RatePolicySaveCommand;

impl CommandAction for RatePolicySaveCommand {
    const NAME: &'static str = "ratePolicySave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = RatePolicySaveInput;
    type Output = RatePolicySaveOutput;
    const ROUTE: &'static str = "/api/rate-policies";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        if input.reason.trim().is_empty() {
            return Err(V3Error::new(
                "validationFailed",
                Some("reason"),
                "reason cannot be empty",
                StatusCode::BAD_REQUEST,
            ));
        }

        let buy_pct = Decimal::from_str(&input.buy_adjust_pct).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("buyAdjustPct"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let sell_pct = Decimal::from_str(&input.sell_adjust_pct).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("sellAdjustPct"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let stale_limit = match input.stale_limit_thb.as_deref() {
            Some(s) => Some(Decimal::from_str(s).map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("staleLimitThb"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?),
            None => None,
        };

        // Query current version with valid_to IS NULL
        let cur_row: Option<CurrentPolicyRow> = sqlx::query_as(
            "SELECT version, guards, max_reference_age_hours, on_stale, stale_limit_thb \
             FROM rate_policy_versions \
             WHERE branch_id = $1 AND valid_to IS NULL \
             FOR UPDATE",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        // Without an EXECUTIVE row here (i.e. a RATE_MANAGER), only the adjustments may change (#1030 §3.9):
        if !may_change_the_frame(who, branch, Utc::now()) {
            let Some(cur) = &cur_row else {
                return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
            };

            if input.guards != cur.guards
                || input.max_reference_age_hours != cur.max_reference_age_hours
                || input.on_stale != cur.on_stale
                || stale_limit != cur.stale_limit_thb
            {
                return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
            }
        }

        let cur_max_ver: Option<(i32,)> = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) FROM rate_policy_versions WHERE branch_id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let next_ver = cur_max_ver.map(|r| r.0).unwrap_or(0) + 1;
        let source_ver = cur_row.as_ref().map(|r| r.version);
        let id = Uuid::new_v4();

        // Update valid_to on previous version
        sqlx::query(
            "UPDATE rate_policy_versions SET valid_to = now() WHERE branch_id = $1 AND valid_to IS NULL",
        )
        .bind(branch.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        sqlx::query(
            "INSERT INTO rate_policy_versions \
             (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
              max_reference_age_hours, on_stale, stale_limit_thb, valid_from, \
              approved_by, reason, source_version) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now(), $10, $11, $12)",
        )
        .bind(id)
        .bind(branch.as_str())
        .bind(next_ver)
        .bind(buy_pct)
        .bind(sell_pct)
        .bind(&input.guards)
        .bind(input.max_reference_age_hours)
        .bind(&input.on_stale)
        .bind(stale_limit)
        .bind(&who.user_id)
        .bind(&input.reason)
        .bind(source_ver)
        .execute(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        Ok(RatePolicySaveOutput {
            id: id.to_string(),
            version: next_ver,
        })
    }
}

/// The guards and the stale settings may change iff the caller has a valid-now EXECUTIVE row covering `branch`
/// (#1030 §3.9; design §3.3: the old owner is EXECUTIVE + ADMIN).
fn may_change_the_frame(who: &Actor, branch: &BranchId, now: DateTime<Utc>) -> bool {
    who.roles.iter().any(|r| {
        r.role == Role::Executive
            && r.valid_from <= now
            && r.valid_to.is_none_or(|to| now < to)
            && r.branch.as_ref().is_none_or(|b| b == branch)
    })
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(RatePolicySaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RatePolicySaveInput::export_all(&cfg)?;
    RatePolicySaveOutput::export_all(&cfg)?;
    Ok(())
}
