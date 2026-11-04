//! `denominationNoteAdd` (`docs/design/board-currencies.md` §7): adds a face value to a currency's group. Add-only.
//! Same permission as `denominationGroupCreate`. SKELETON: `run_as` is BC-11's.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::currency::denomination_group_service::DenominationGroupService;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::infrastructure::currency::denomination_groups::PgDenominationGroups;

use super::commands::map_rate_engine_error;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::caller::Caller;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denomination_note_add.ts")]
pub struct DenominationNoteAddInput {
    pub currency: String,
    pub group: String,
    /// A positive decimal string, at most 4 places.
    pub face_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "denomination_note_add.ts")]
pub struct DenominationNoteAddOutput {
    #[ts(type = "number")]
    pub id: i64,
    pub currency: String,
    pub group: String,
    pub face_value: String,
}

pub struct DenominationNoteAddCommand {
    service: Arc<DenominationGroupService>,
}

impl DenominationNoteAddCommand {
    pub fn new(service: Arc<DenominationGroupService>) -> Self {
        Self { service }
    }
}

impl CommandAction for DenominationNoteAddCommand {
    const NAME: &'static str = "denominationNoteAdd";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "denominationGroupCreate");
    type Input = DenominationNoteAddInput;
    type Output = DenominationNoteAddOutput;
    const ROUTE: &'static str = "/api/denomination-notes";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Err(V3Error::from(PolicyError::SignIn))
    }

    async fn run_as(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        caller: &Caller,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let who = caller
            .user
            .as_ref()
            .ok_or_else(|| V3Error::from(PolicyError::SignIn))?;
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

        let id = self
            .service
            .add_note(tx, who, code, &group, &input.face_value)
            .await
            .map_err(map_rate_engine_error)?;

        Ok(DenominationNoteAddOutput {
            id,
            currency: code.as_str().to_string(),
            group: group.as_str().to_string(),
            face_value: input.face_value,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let groups = Arc::new(PgDenominationGroups::new(state.db.clone()));
    let audit = Arc::new(PgAuditLog::new(state.db.clone()));
    let service = Arc::new(DenominationGroupService::new(groups, audit));
    CommandEndpoint::register(cfg, Arc::new(DenominationNoteAddCommand::new(service)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    DenominationNoteAddInput::export_all(&cfg)?;
    DenominationNoteAddOutput::export_all(&cfg)?;
    Ok(())
}
