//! Rate reference fetch command (§4.7, §6).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_rate::application::reference::reference_batch_writer::ReferenceBatchWriter;
use xc_rate::infrastructure::reference::pg_reference_batch_store::PgReferenceBatchStore;

use super::commands::map_rate_engine_error;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_reference_fetch.ts")]
pub struct RateReferenceFetchInput {}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "rate_reference_fetch.ts")]
pub struct RateReferenceFetchOutput {
    pub batch_id: String,
}

pub struct RateReferenceFetchCommand {
    writer: Arc<ReferenceBatchWriter>,
}

impl RateReferenceFetchCommand {
    pub fn new(writer: Arc<ReferenceBatchWriter>) -> Self {
        Self { writer }
    }
}

impl CommandAction for RateReferenceFetchCommand {
    const NAME: &'static str = "rateReferenceFetch";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "rateDraft");
    type Input = RateReferenceFetchInput;
    type Output = RateReferenceFetchOutput;
    const ROUTE: &'static str = "/api/rate-references/fetch";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let batch_id = self
            .writer
            .fetch_and_write(tx)
            .await
            .map_err(map_rate_engine_error)?;

        Ok(RateReferenceFetchOutput {
            batch_id: batch_id.to_string(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let source = state.rate_source.clone();
    let store = Arc::new(PgReferenceBatchStore::new());
    let writer = Arc::new(ReferenceBatchWriter::new(source, store));
    CommandEndpoint::register(cfg, Arc::new(RateReferenceFetchCommand::new(writer)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    RateReferenceFetchInput::export_all(&cfg)?;
    RateReferenceFetchOutput::export_all(&cfg)?;
    Ok(())
}
