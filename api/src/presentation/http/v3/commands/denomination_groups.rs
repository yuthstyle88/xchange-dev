//! Denomination groups commands (§2, §3.2, §6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Standing};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::currency::denomination_group_service::DenominationGroupService;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::infrastructure::currency::denomination_groups::PgDenominationGroups;

use super::commands::map_rate_engine_error;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, role_not_allowed_here,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denomination_groups.ts")]
pub struct DenominationGroupCreateInput {
    pub currency: String,
    pub group: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denomination_groups.ts")]
pub struct DenominationGroupCreateOutput {
    pub currency: String,
    pub group: String,
}

pub struct DenominationGroupCreateCommand {
    service: Arc<DenominationGroupService>,
}

impl DenominationGroupCreateCommand {
    pub fn new(service: Arc<DenominationGroupService>) -> Self {
        Self { service }
    }
}

impl CommandAction for DenominationGroupCreateCommand {
    const NAME: &'static str = "denominationGroupCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = DenominationGroupCreateInput;
    type Output = DenominationGroupCreateOutput;
    const ROUTE: &'static str = "/api/denomination-groups";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Standing::of(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
        let code = CurrencyCode::parse(&input.currency).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("currency"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let group = GroupCode::new(&input.group).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("group"),
                e,
                StatusCode::BAD_REQUEST,
            )
        })?;

        self.service
            .create_group(tx, who, code, &group)
            .await
            .map_err(map_rate_engine_error)?;

        Ok(DenominationGroupCreateOutput {
            currency: code.as_str().to_string(),
            group: group.as_str().to_string(),
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denomination_groups.ts")]
pub struct DenominationGroupMapInput {
    pub currency: String,
    pub group: String,
    pub provider: String,
    pub instrument_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denomination_groups.ts")]
pub struct DenominationGroupMapOutput {
    pub currency: String,
    pub group: String,
    pub provider: String,
    pub instrument_key: String,
}

pub struct DenominationGroupMapCommand {
    service: Arc<DenominationGroupService>,
}

impl DenominationGroupMapCommand {
    pub fn new(service: Arc<DenominationGroupService>) -> Self {
        Self { service }
    }
}

impl CommandAction for DenominationGroupMapCommand {
    const NAME: &'static str = "denominationGroupMap";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = DenominationGroupMapInput;
    type Output = DenominationGroupMapOutput;
    const ROUTE: &'static str = "/api/denomination-groups/map";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Standing::of(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
        let code = CurrencyCode::parse(&input.currency).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("currency"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let group = GroupCode::new(&input.group).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("group"),
                e,
                StatusCode::BAD_REQUEST,
            )
        })?;

        self.service
            .map_group(
                tx,
                who,
                code,
                &group,
                &input.provider,
                &input.instrument_key,
            )
            .await
            .map_err(map_rate_engine_error)?;

        Ok(DenominationGroupMapOutput {
            currency: code.as_str().to_string(),
            group: group.as_str().to_string(),
            provider: input.provider,
            instrument_key: input.instrument_key,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denomination_groups.ts")]
pub struct DenominationGroupRemoveInput {
    pub currency: String,
    pub group: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denomination_groups.ts")]
pub struct DenominationGroupRemoveOutput {
    pub currency: String,
    pub group: String,
}

pub struct DenominationGroupRemoveCommand {
    service: Arc<DenominationGroupService>,
}

impl DenominationGroupRemoveCommand {
    pub fn new(service: Arc<DenominationGroupService>) -> Self {
        Self { service }
    }
}

impl CommandAction for DenominationGroupRemoveCommand {
    const NAME: &'static str = "denominationGroupRemove";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = DenominationGroupRemoveInput;
    type Output = DenominationGroupRemoveOutput;
    const ROUTE: &'static str = "/api/denomination-groups/remove";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Standing::of(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
        let code = CurrencyCode::parse(&input.currency).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("currency"),
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )
        })?;
        let group = GroupCode::new(&input.group).map_err(|e| {
            V3Error::new(
                "validationFailed",
                Some("group"),
                e,
                StatusCode::BAD_REQUEST,
            )
        })?;

        self.service
            .remove_group(tx, who, code, &group)
            .await
            .map_err(map_rate_engine_error)?;

        Ok(DenominationGroupRemoveOutput {
            currency: code.as_str().to_string(),
            group: group.as_str().to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let groups = Arc::new(PgDenominationGroups::new(state.db.clone()));
    let audit = Arc::new(PgAuditLog::new(state.db.clone()));
    let service = Arc::new(DenominationGroupService::new(groups, audit));

    CommandEndpoint::register(
        cfg,
        Arc::new(DenominationGroupCreateCommand::new(service.clone())),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(DenominationGroupMapCommand::new(service.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(DenominationGroupRemoveCommand::new(service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DenominationGroupCreateInput::export_all(&cfg)?;
    DenominationGroupCreateOutput::export_all(&cfg)?;
    DenominationGroupMapInput::export_all(&cfg)?;
    DenominationGroupMapOutput::export_all(&cfg)?;
    DenominationGroupRemoveInput::export_all(&cfg)?;
    DenominationGroupRemoveOutput::export_all(&cfg)?;
    Ok(())
}
