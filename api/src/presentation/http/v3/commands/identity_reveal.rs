//! K8: `POST /api/customers/{id}/identities/{identityId}/reveal` (`identityReveal`, design
//! `customer-identity-consent.md` §4.3, §4.4). Body `{"branch", "reason"}`; `reason` is required.
//!
//! The answer holds the plain value, so the idempotency record keeps only `identityId` and `type`
//! (`stored_output`): a replay never answers the value again, and no value is written to `idempotency_records`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

/// The longest `reason` accepted, in characters.
pub const MAX_REASON_CHARS: usize = 500;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRevealInput {
    pub id: String,
    pub identity_id: Uuid,
    pub branch: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRevealOutput {
    pub identity_id: Uuid,
    #[serde(rename = "type")]
    pub identity_type: String,
    pub value: String,
}

pub struct IdentityRevealCommand {
    pool: PgPool,
}

impl IdentityRevealCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CommandAction for IdentityRevealCommand {
    const NAME: &'static str = "identityReveal";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "identityReveal");
    type Input = IdentityRevealInput;
    type Output = IdentityRevealOutput;
    const ROUTE: &'static str = "/api/customers/{id}/identities/{identityId}/reveal";
    const METHOD: Method = Method::POST;

    fn stored_output(&self, output: &Self::Output) -> serde_json::Value {
        let _ = output;
        todo!("K8: keep identityId and type only")
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let _ = (&self.pool, tx, branch, who, input);
        todo!("K8: validate reason, CustomerService::reveal_in")
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(cfg, Arc::new(IdentityRevealCommand::new(state.db.clone())));
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
