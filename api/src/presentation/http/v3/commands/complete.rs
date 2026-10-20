//! TX-5 #873: `POST /api/transactions/{id}/complete` (design `quote-transaction.md` §4.2, §4.5): a CONFIRMED
//! exchange is completed by `CompleteExchange` in the command's one transaction. It inherits `(Rpc,
//! "saveTransaction")`, as TX-4's confirm and cancel do.
//!
//! The posting is the ledger's `LedgerCashPostings` (LG-4 #929, design §6.1).

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
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
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
    CommandAction, CommandEndpoint, out_of_scope, role_not_allowed_here, server_error,
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
fn save_actor(who: &Actor) -> Result<SaveActor, V3Error> {
    SaveActor::from_actor(who).ok_or_else(role_not_allowed_here)
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
    pool: PgPool,
    approvals: Arc<xc_approval::application::approval_service::ApprovalService>,
    service: CompleteExchange,
}

impl TransactionCompleteCommand {
    pub fn new(
        pool: PgPool,
        files: FileService,
        steps: Vec<Arc<dyn SaveStep>>,
        effects: ExchangeEffects,
    ) -> Self {
        let approvals = crate::presentation::http::v3::commands::make_approval_service(pool.clone());
        Self {
            pool: pool.clone(),
            approvals,
            service: CompleteExchange::new(
                pool,
                files,
                Arc::new(LedgerCashPostings::new()),
                steps,
                Arc::new(effects),
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
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let id = this_branchs(tx, branch, &input.id).await?;
        let actor = save_actor(who)?;
        let done = match self.service.run_in(tx, id, &actor, Utc::now()).await {
            Ok(done) => done,
            Err(AppError::Domain(m)) if m.starts_with("stockInsufficient") => {
                use crate::application::workflows::exchange::negative_stock_gate::{
                    NegativeStockGate, NegativeStockOutcome,
                };
                let outcome = NegativeStockGate::open_after_refusal(
                    &self.pool,
                    &self.approvals,
                    id,
                    who,
                    key.as_str(),
                )
                .await
                .map_err(|_| server_error())?;
                return Err(match outcome {
                    NegativeStockOutcome::ApprovalRequired(req_id) => V3Error::new(
                        "approvalRequired",
                        None,
                        format!("approvalRequired: {req_id}"),
                        StatusCode::CONFLICT,
                    ),
                    NegativeStockOutcome::Rejected(req_id) => V3Error::new(
                        "stockInsufficient",
                        None,
                        format!(
                            "stockInsufficient: negative stock approval request {req_id} was rejected"
                        ),
                        StatusCode::CONFLICT,
                    ),
                });
            }
            Err(e) => return Err(complete_refusal(e)),
        };
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
            crate::bootstrap::exchange_effects::effects(state),
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
