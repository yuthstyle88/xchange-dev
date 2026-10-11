//! TX-5 #873: `POST /api/transactions/{id}/complete` (design `quote-transaction.md` §4.2, §4.5): a CONFIRMED
//! exchange is completed by `CompleteExchange` in the command's one transaction. It inherits `(Rpc,
//! "saveTransaction")`, as TX-4's confirm and cancel do.
//!
//! Until LG-4's cutover the posting is `InterimCashPostings` (design §6.1, #873 ruling Q9).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_cash::infrastructure::interim_postings::InterimCashPostings;
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_kernel::ids::BranchId;

use super::transactions::{refusal, save_steps};
use crate::application::workflows::exchange::complete::{CompleteExchange, ExchangeEffects};
use crate::application::workflows::save_step::{SaveActor, SaveStep};
use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::FileService;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::transaction::TransactionDto;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "complete.ts")]
pub struct TransactionCompleteInput {
    /// The path's `{id}`.
    pub id: String,
}

/// The receipt a completion issued: its number (the exchange's `documentRef`) and revision 1.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "complete.ts")]
pub struct CompletedReceiptDto {
    pub number: String,
    pub revision: i32,
}

/// The completed exchange as TX-4's `TransactionDto`, with its receipt beside it (#873 §8).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "complete.ts")]
pub struct TransactionCompletedDto {
    #[serde(flatten)]
    pub transaction: TransactionDto,
    pub receipt: CompletedReceiptDto,
}

/// `stockInsufficient` is a conflict with the drawer as it stands (409, #873 ruling Q9); every other refusal is
/// TX-4's answer.
fn complete_refusal(e: AppError) -> V3Error {
    if let AppError::Domain(m) = &e
        && m.starts_with("stockInsufficient")
    {
        return V3Error::new("stockInsufficient", None, m.clone(), StatusCode::CONFLICT);
    }
    refusal(e)
}

/// The signed-in caller as the exchange workflows are told who acts.
fn save_actor(who: &Actor) -> SaveActor {
    SaveActor {
        id: who.id().to_owned(),
        username: who.id().to_owned(),
        employee_id: (who.role() != Role::Owner).then(|| who.id().to_owned()),
        role: who.role(),
    }
}

/// The exchange named by the path, after checking it is this branch's.
async fn this_branchs(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    id: &str,
) -> Result<TransactionId, V3Error> {
    let id = Uuid::parse_str(id).map(TransactionId).map_err(|_| {
        V3Error::new(
            "validationFailed",
            Some("id"),
            "invalid transaction id",
            StatusCode::BAD_REQUEST,
        )
    })?;
    let owner: Option<String> = sqlx::query_scalar("SELECT branch_id FROM transactions WHERE id = $1")
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?;
    match owner {
        None => Err(V3Error::new(
            "notFound",
            Some("id"),
            "no such transaction",
            StatusCode::NOT_FOUND,
        )),
        Some(owner) if owner != branch.as_str() => Err(out_of_scope()),
        Some(_) => Ok(id),
    }
}

pub struct TransactionCompleteCommand {
    service: CompleteExchange,
}

impl TransactionCompleteCommand {
    pub fn new(pool: PgPool, files: FileService, steps: Vec<Arc<dyn SaveStep>>) -> Self {
        Self {
            service: CompleteExchange::new(
                pool,
                files,
                Arc::new(InterimCashPostings::new()),
                steps,
                Arc::new(ExchangeEffects::new(Vec::new())),
            ),
        }
    }
}

impl CommandAction for TransactionCompleteCommand {
    const NAME: &'static str = "transactionComplete";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "saveTransaction");
    type Input = TransactionCompleteInput;
    type Output = TransactionCompletedDto;
    const ROUTE: &'static str = "/api/transactions/{id}/complete";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let id = this_branchs(tx, branch, &input.id).await?;
        let done = self
            .service
            .run_in(tx, id, &save_actor(who), Utc::now())
            .await
            .map_err(complete_refusal)?;
        Ok(TransactionCompletedDto {
            transaction: TransactionDto::from_domain(&done.transaction),
            receipt: CompletedReceiptDto {
                number: done.receipt.number,
                revision: done.receipt.revision,
            },
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    let files = FileService::new(pool.clone(), state.settings.files.dir.clone());
    CommandEndpoint::register(
        cfg,
        Arc::new(TransactionCompleteCommand::new(
            pool,
            files,
            save_steps(state),
        )),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    TransactionCompleteInput::export_all(&cfg)?;
    CompletedReceiptDto::export_all(&cfg)?;
    TransactionCompletedDto::export_all(&cfg)?;
    Ok(())
}
