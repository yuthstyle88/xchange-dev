//! OR-5a (#945): `PUT /api/branches/profile` (`branchSave`). A04: save a branch's profile (`BranchService::save_profile`, OR-1's columns).
//!
//! The seam names the branch in the body's `branch` and already holds its `BranchTx`, so the
//! profile is written by `BranchService::save_profile_in` in the seam's own transaction.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use pos_error::AppError;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use pos_infra::transactions::Transactor;
use xc_rate::application::branch::service::{BranchProfile, BranchService};

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, out_of_scope, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::dto::v3::admin_branches_profile::AdminBranchProfileData;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_save.ts")]
pub struct BranchSaveInput {
    pub code: String,
    pub address: Option<String>,
    pub timezone: String,
    #[ts(type = "Record<string, Array<{ open: string; close: string }>> | null")]
    pub opening_hours: Option<serde_json::Value>,
    pub booking_enabled: bool,
}

pub struct BranchSaveCommand(PgPool);

impl CommandAction for BranchSaveCommand {
    const NAME: &'static str = "branchSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "branchSave");
    type Input = BranchSaveInput;
    type Output = AdminBranchProfileData;
    const ROUTE: &'static str = "/api/branches/profile";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        // #945 ruling 1: a branch of another organisation is out of the caller's scope. An old cookie's actor names
        // its organisation by code ("XC"), an Identity session's by id.
        let ours: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM branches b JOIN organizations o ON o.id = b.organization_id \
             WHERE b.id = $1 AND (o.id::text = $2 OR o.code = $2))",
        )
        .bind(branch.as_str())
        .bind(&who.organization_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| server_error())?;
        if !ours {
            return Err(out_of_scope());
        }
        let branches = BranchService::new(BranchTx::new(Transactor::new(self.0.clone())));
        let profile = BranchProfile {
            code: input.code,
            address: input.address,
            timezone: input.timezone,
            opening_hours: input.opening_hours,
            booking_enabled: input.booking_enabled,
        };
        let saved = branches
            .save_profile_in(tx, branch, profile)
            .await
            .map_err(|e| match e {
                AppError::Validation(message) | AppError::Domain(message) => V3Error::new(
                    "validationFailed",
                    None,
                    message,
                    StatusCode::BAD_REQUEST,
                ),
                AppError::Conflict(message) => {
                    V3Error::new("conflict", Some("code"), message, StatusCode::CONFLICT)
                }
                other => {
                    tracing::error!(error = %other, "branchSave failed");
                    server_error()
                }
            })?;
        Ok(AdminBranchProfileData {
            branch: saved.id.as_str().to_owned(),
            name: saved.name,
            code: saved.code,
            address: saved.address,
            timezone: saved.timezone,
            opening_hours: saved.opening_hours,
            booking_enabled: saved.booking_enabled,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(BranchSaveCommand(state.db.clone())));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchSaveInput::export_all(&cfg)?;
    AdminBranchProfileData::export_all(&cfg)?;
    Ok(())
}
