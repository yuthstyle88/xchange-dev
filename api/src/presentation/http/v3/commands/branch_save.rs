//! OR-5a (#945): `PUT /api/branches/profile` (`branchSave`). A04: save a branch's profile (`BranchService::save_profile`, OR-1's columns).
//!
//! The seam names the branch in the body's `branch` and already holds its `BranchTx`, so the
//! profile is written by `BranchService::save_profile_in` in the seam's own transaction.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::{BranchProfile, BranchService};
use xc_rate::domain::branch::branch_hours::BranchHours;
use xc_rate::domain::branch::contact_channel::{BranchStatus, ContactChannel};

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::admin_branches_profile::{
    AdminBranchContactChannel, AdminBranchProfileData,
};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input::{self, TextBound};

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "branch_save.ts")]
pub struct BranchSaveContactChannelInput {
    #[ts(type = "\"PHONE\" | \"LINE\" | \"EMAIL\" | \"FACEBOOK\" | \"WEBSITE\"")]
    pub kind: String,
    pub value: String,
}

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
    #[serde(default)]
    #[ts(optional)]
    pub contact_channels: Option<Vec<BranchSaveContactChannelInput>>,
    #[serde(default)]
    #[ts(optional, type = "\"ACTIVE\" | \"INACTIVE\"")]
    pub status: Option<String>,
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
        // #945 ruling 1: a branch of another organisation is out of the caller's scope.
        let ours: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM branches b JOIN organizations o ON o.id = b.organization_id \
             WHERE b.id = $1 AND o.id::text = $2)",
        )
        .bind(branch.as_str())
        .bind(&who.organization_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| server_error())?;
        if !ours {
            return Err(out_of_scope());
        }
        input::text("code", &input.code, TextBound { min: 1, max: 40 })?;
        if let Some(ref addr) = input.address {
            input::text("address", addr, TextBound { min: 0, max: 200 })?;
        }
        let contact_channels = match input.contact_channels {
            Some(raw) => {
                let parsed: Vec<ContactChannel> = raw
                    .into_iter()
                    .map(|ch| ContactChannel::parse(&ch.kind, &ch.value))
                    .collect::<Result<_, _>>()
                    .map_err(|e| {
                        V3Error::new(
                            "validationFailed",
                            Some("contactChannels"),
                            e.to_string(),
                            StatusCode::BAD_REQUEST,
                        )
                    })?;
                Some(ContactChannel::validate_list(&parsed).map_err(|e| {
                    V3Error::new(
                        "validationFailed",
                        Some("contactChannels"),
                        e.to_string(),
                        StatusCode::BAD_REQUEST,
                    )
                })?)
            }
            None => None,
        };
        let status = match input.status {
            Some(raw) => Some(BranchStatus::parse(&raw).map_err(|e| {
                V3Error::new(
                    "validationFailed",
                    Some("status"),
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?),
            None => None,
        };
        let branches = BranchService::new(BranchTx::new(Transactor::new(self.0.clone())));
        let profile_field =
            bad_profile_field(&input.code, &input.timezone, input.opening_hours.as_ref());
        let profile = BranchProfile {
            code: input.code,
            address: input.address,
            timezone: input.timezone,
            opening_hours: input.opening_hours,
            booking_enabled: input.booking_enabled,
        };
        let saved = branches
            .save_profile_with_contact_in(tx, branch, profile, contact_channels, status)
            .await
            .map_err(|e| match e {
                AppError::Validation(message) | AppError::Domain(message) => V3Error::new(
                    "validationFailed",
                    profile_field,
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
            contact_channels: saved
                .contact_channels
                .into_iter()
                .map(|ch| AdminBranchContactChannel {
                    kind: ch.kind.as_str().to_owned(),
                    value: ch.value,
                })
                .collect(),
            status: saved.status.as_str().to_owned(),
        })
    }
}

fn bad_profile_field(
    code: &str,
    timezone: &str,
    opening_hours: Option<&serde_json::Value>,
) -> Option<&'static str> {
    if code.trim().is_empty() || BranchHours::new(timezone, opening_hours).is_ok() {
        return None;
    }
    if BranchHours::new(timezone, None).is_err() {
        return Some("timezone");
    }
    for (day, val) in opening_hours?.as_object()? {
        if BranchHours::new(timezone, Some(&serde_json::json!({ day: val }))).is_err() {
            return match day.as_str() {
                "mon" => Some("hours.mon"),
                "tue" => Some("hours.tue"),
                "wed" => Some("hours.wed"),
                "thu" => Some("hours.thu"),
                "fri" => Some("hours.fri"),
                "sat" => Some("hours.sat"),
                "sun" => Some("hours.sun"),
                _ => None,
            };
        }
    }
    None
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
