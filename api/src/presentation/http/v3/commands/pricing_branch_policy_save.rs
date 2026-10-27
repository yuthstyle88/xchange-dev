//! `pricingBranchPolicySave` (pricing-tiers.md §15.4; `H:` §7): how a branch follows MAIN — `buyBp`/`sellBp`
//! (−300..=300) for the branch default (`"*"`) or one currency, `own` and `follow` for one currency. An EXECUTIVE frame
//! change (`may_change_the_frame`), V.9's OWNER.
//!
//! SKELETON (XC-V9-PRICING): `run` is PV-d2's. Types, route and registration stay.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::domain::pricing::branch_policy::{
    BranchPolicyPatch, BranchPolicyStore, BranchPricingPolicy,
};
use xc_rate::infrastructure::pricing::branch_policy_store::PgBranchPolicyStore;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_branch_policy_save.ts")]
pub struct PricingBranchPolicySaveInput {
    /// `"*"` for the branch default, or a currency code.
    pub currency: String,
    /// Basis points, −300..=300; omitted leaves the stored value.
    #[ts(optional)]
    pub buy_bp: Option<i32>,
    #[ts(optional)]
    pub sell_bp: Option<i32>,
    /// The branch prices this currency itself (a currency row only).
    #[ts(optional)]
    pub own: Option<bool>,
    /// Enrol this currency in MAIN's FOLLOW (a currency row only; `H:` §7).
    #[ts(optional)]
    pub follow: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "pricing_branch_policy_save.ts")]
pub struct PricingBranchPolicySaveOutput {
    pub saved: bool,
}

pub struct PricingBranchPolicySaveCommand;

impl CommandAction for PricingBranchPolicySaveCommand {
    const NAME: &'static str = "pricingBranchPolicySave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ratePolicySave");
    type Input = PricingBranchPolicySaveInput;
    type Output = PricingBranchPolicySaveOutput;
    const ROUTE: &'static str = "/api/pricing/branch-policy";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let trimmed = input.currency.trim();
        let scope: &str = if trimmed == "*" {
            "*"
        } else {
            CurrencyCode::parse(trimmed)
                .map_err(|e| {
                    V3Error::new(
                        "validationFailed",
                        Some("currency"),
                        e.to_string(),
                        StatusCode::BAD_REQUEST,
                    )
                })?
                .as_str()
        };

        if scope == "*" {
            if input.own.is_some() {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("own"),
                    "own is only allowed on a currency row",
                    StatusCode::BAD_REQUEST,
                ));
            }
            if input.follow.is_some() {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("follow"),
                    "follow is only allowed on a currency row",
                    StatusCode::BAD_REQUEST,
                ));
            }
        }

        let buy_bp = input
            .buy_bp
            .map(BranchPricingPolicy::check_bp)
            .transpose()
            .map_err(|msg| {
                V3Error::new(
                    "validationFailed",
                    Some("buyBp"),
                    msg,
                    StatusCode::BAD_REQUEST,
                )
            })?;
        let sell_bp = input
            .sell_bp
            .map(BranchPricingPolicy::check_bp)
            .transpose()
            .map_err(|msg| {
                V3Error::new(
                    "validationFailed",
                    Some("sellBp"),
                    msg,
                    StatusCode::BAD_REQUEST,
                )
            })?;

        let now = Utc::now();
        if !crate::presentation::http::v3::rate_policy_save::may_change_the_frame(who, branch, now)
        {
            return Err(V3Error::from(PolicyError::RoleNotAllowedHere));
        }

        PgBranchPolicyStore
            .save_in(
                tx,
                branch.as_str(),
                scope,
                BranchPolicyPatch {
                    buy_bp,
                    sell_bp,
                    own: input.own,
                    follow: input.follow,
                },
                &who.user_id,
                now,
            )
            .await
            .map_err(|_| server_error())?;

        Ok(PricingBranchPolicySaveOutput { saved: true })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(PricingBranchPolicySaveCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    PricingBranchPolicySaveInput::export_all(&cfg)?;
    PricingBranchPolicySaveOutput::export_all(&cfg)?;
    Ok(())
}
