//! EX-5 (#1075): `POST /api/v3/commands/branch_alert_settings_save` saves a branch's alert settings.

use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_approval::domain::audit_log::AuditEvent;
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::executive::alert_settings_repository::{
    PgAlertSettingsRepository, SaveAlertSettingsParams,
};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::branch_alert_settings::{
    BranchAlertSettingsData, BranchAlertSettingsSaveInput,
};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

pub struct BranchAlertSettingsSaveCommand {
    pool: PgPool,
}

impl BranchAlertSettingsSaveCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for BranchAlertSettingsSaveCommand {
    const NAME: &'static str = "branchAlertSettingsSave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "branchAlertSettingsSave");
    type Input = BranchAlertSettingsSaveInput;
    type Output = BranchAlertSettingsData;
    const ROUTE: &'static str = "/api/v3/commands/branch_alert_settings_save";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        // Step 1: Validation
        if input.stale_data_minutes.is_some_and(|m| m <= 0) {
            return Err(V3Error::new(
                "validationFailed",
                Some("staleDataMinutes"),
                "staleDataMinutes must be greater than 0",
                StatusCode::BAD_REQUEST,
            ));
        }

        let mut seen = HashSet::new();
        for floor in &input.floors {
            if !seen.insert(&floor.currency) {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("floors"),
                    "duplicate currency",
                    StatusCode::BAD_REQUEST,
                ));
            }
            let dec = Decimal::from_str(&floor.min_quantity).map_err(|_| {
                V3Error::new(
                    "validationFailed",
                    Some("floors"),
                    "malformed minQuantity",
                    StatusCode::BAD_REQUEST,
                )
            })?;
            if dec.is_sign_negative() && !dec.is_zero() {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("floors"),
                    "minQuantity must be non-negative",
                    StatusCode::BAD_REQUEST,
                ));
            }
            if dec.scale() > 4 {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("floors"),
                    "minQuantity scale exceeds 4 decimal places",
                    StatusCode::BAD_REQUEST,
                ));
            }
        }

        if !input.floors.is_empty() {
            let codes: Vec<&str> = input.floors.iter().map(|f| f.currency.as_str()).collect();
            let existing: Vec<String> = sqlx::query_scalar(
                "SELECT code FROM currencies WHERE code = ANY($1)",
            )
            .bind(&codes)
            .fetch_all(&mut **tx)
            .await
            .map_err(|_| server_error())?;

            for code in &codes {
                if !existing.iter().any(|c| c.trim() == *code) {
                    return Err(V3Error::new(
                        "validationFailed",
                        Some("floors"),
                        format!("currency {code} not in currencies"),
                        StatusCode::BAD_REQUEST,
                    ));
                }
            }
        }

        // Step 2: Org & User ID
        let org_id: Uuid = sqlx::query_scalar(
            "SELECT organization_id FROM branches WHERE id = $1",
        )
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        let user_id = Uuid::parse_str(who.id()).map_err(|_| server_error())?;

        // Step 3: Version check
        let repo = PgAlertSettingsRepository::new(self.pool.clone());
        let before_data = repo
            .read_in(tx, org_id, branch)
            .await
            .map_err(|_| server_error())?;

        let current_version = before_data.version;
        let is_match = match (current_version, input.expected_version) {
            (None, None) => true,
            (Some(c), Some(e)) if c == e => true,
            _ => false,
        };

        if !is_match {
            return Err(V3Error::new(
                "staleVersion",
                None,
                "stale version",
                StatusCode::CONFLICT,
            ));
        }

        // Step 4: Write
        let new_version = current_version.map(|v| v + 1).unwrap_or(1);
        let after_data = repo
            .save_in(
                tx,
                SaveAlertSettingsParams {
                    org_id,
                    branch_id: branch,
                    stale_data_minutes: input.stale_data_minutes,
                    floors: &input.floors,
                    new_version,
                    updated_by: user_id,
                },
            )
            .await
            .map_err(|_| server_error())?;

        // Step 5: Audit
        let before = if current_version.is_some() {
            Some(serde_json::to_value(&before_data).map_err(|_| server_error())?)
        } else {
            None
        };
        let after = Some(serde_json::to_value(&after_data).map_err(|_| server_error())?);

        let audit_log = PgAuditLog::new(self.pool.clone());
        audit_log
            .append(
                tx,
                &AuditEvent {
                    actor_id: who.id().to_owned(),
                    actor_role: who.role().as_str().to_owned(),
                    scope: "branch".to_owned(),
                    action: "BRANCH_ALERT_SETTINGS_SAVE".to_owned(),
                    object_ref: format!("branch:{}", branch.as_str()),
                    branch_id: Some(branch.as_str().to_owned()),
                    before,
                    after,
                },
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "branch alert settings audit failed");
                server_error()
            })?;

        Ok(after_data)
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(BranchAlertSettingsSaveCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchAlertSettingsSaveInput::export_all(&cfg)?;
    Ok(())
}
