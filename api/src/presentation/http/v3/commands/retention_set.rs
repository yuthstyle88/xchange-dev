//! Retention set command (§4.4, §6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::retention_job::RetentionJob;
use xc_customer::domain::consent::ConsentError;
use xc_customer::domain::retention::DataType;
use xc_kernel::ids::BranchId;
use xc_kernel::money::IntBound;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "retention_set.ts")]
pub struct RetentionSetInput {
    pub data_type: String,
    pub period_days: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "retention_set.ts")]
pub struct RetentionSetOutput {
    pub data_type: String,
    pub period_days: Option<i32>,
}

pub struct RetentionSetCommand;

impl CommandAction for RetentionSetCommand {
    const NAME: &'static str = "retentionSet";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "consentPolicy");
    type Input = RetentionSetInput;
    type Output = RetentionSetOutput;
    const ROUTE: &'static str = "/api/retention-policies/{dataType}";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let data_type: DataType = input.data_type.parse().map_err(|_| {
            V3Error::new(
                "validationFailed",
                Some("dataType"),
                "unknown data type",
                StatusCode::BAD_REQUEST,
            )
        })?;
        if let Some(days) = input.period_days {
            input::integer("periodDays", i64::from(days), IntBound::integer(1, 3_650))?;
        }

        let org_id: Option<Uuid> =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|_| server_error())?;

        let org_id = org_id.ok_or_else(server_error)?;

        let set_by = Uuid::parse_str(&who.user_id).ok().map(|u| u.to_string());
        let now = Utc::now();

        RetentionJob::set_period_in(
            tx,
            &org_id.to_string(),
            set_by.as_deref(),
            data_type,
            input.period_days,
            now,
        )
        .await
        .map_err(|e| match e {
            AppError::Domain(msg) if msg == ConsentError::InvalidPeriod.to_string() => {
                V3Error::new(
                    "validationFailed",
                    Some("periodDays"),
                    "period must be greater than zero",
                    StatusCode::BAD_REQUEST,
                )
            }
            _ => server_error(),
        })?;

        Ok(RetentionSetOutput {
            data_type: data_type.to_string(),
            period_days: input.period_days,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(RetentionSetCommand));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RetentionSetInput::export_all(&cfg)?;
    RetentionSetOutput::export_all(&cfg)?;
    Ok(())
}
