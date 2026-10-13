//! RV-3 #684: `POST /api/reversals/{id}/post` (design `reversal.md` §4.3, §8): an APPROVED reversal of this branch
//! is posted by `PostReversal` in the command's one transaction (the CM-1 seam's `BranchTx`, so a refusal from the
//! posting, an effect, the feed or the mark rolls everything back).
//!
//! It inherits `(Rpc, "reversalRequest")`: design §4.3 gives the post the `ReversalRequest` permission, the row RV-2's
//! request already uses. Flat file: `build.rs` serves only plain files in `commands/` (lane controller's ruling 1 on
//! #684). The answer is RV-2's `ReversalDto` with the document number beside it (ruling 2).

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
use xc_exchange::domain::reversal::repository::ReversalRepository;
use xc_exchange::domain::reversal::reversal::ReversalId;
use xc_exchange::infrastructure::reversal::repository::PgReversalRepository;
use xc_kernel::ids::BranchId;

use crate::application::workflows::exchange::complete::ExchangeEffects;
use crate::application::workflows::exchange::reversal_post::PostReversal;
use crate::application::workflows::save_step::SaveActor;
use crate::bootstrap::AppState;
use crate::infrastructure::files::file_service::FileService;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::reversal::ReversalDto;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, role_not_allowed_here, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "reversal_post.ts")]
pub struct ReversalPostInput {
    /// The path's `{id}`: the reversal to post.
    pub id: String,
}

/// The posted reversal as RV-2's `ReversalDto`, with the reversal document's number beside it (#684 §8, ruling 2).
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "reversal_post.ts")]
pub struct ReversalPostedDto {
    /// RV-2's `ReversalDto` carries no TS of its own (#682's RG7: a command DTO commits none), so its shape is
    /// spelled here; `the_reversal_dto_shape_is_the_one_ts_names` in this file keeps the two the same.
    #[ts(
        type = "{ id: string, status: string, originalTransactionId: string, reason: string, amountThb: string, businessDate: string, approvalRequestId: string, }"
    )]
    pub reversal: ReversalDto,
    /// `<document_ref>-R2`, the number of receipts revision 2 (V-4).
    pub document_number: String,
}

/// Every refusal of §8 on the wire: the state conflicts are 409, as TX-5's `stockInsufficient` (#873 ruling Q9).
fn refusal(e: AppError) -> V3Error {
    let AppError::Domain(m) = e else {
        tracing::error!(error = %e, "reversal post failed");
        return server_error();
    };
    let code = m
        .split(|c: char| c == ':' || c.is_whitespace())
        .next()
        .unwrap_or("");
    let (code, status) = match code {
        "outOfScope" => ("outOfScope", StatusCode::FORBIDDEN),
        "staleVersion" => ("staleVersion", StatusCode::CONFLICT),
        "reversalWindowClosed" => ("reversalWindowClosed", StatusCode::CONFLICT),
        "noOpenShift" => ("noOpenShift", StatusCode::CONFLICT),
        "stockInsufficient" => ("stockInsufficient", StatusCode::CONFLICT),
        "idempotencyConflict" => ("idempotencyConflict", StatusCode::CONFLICT),
        _ => ("validationFailed", StatusCode::BAD_REQUEST),
    };
    V3Error::new(code, None, m, status)
}

/// The signed-in caller as the exchange workflows are told who acts.
fn save_actor(who: &Actor) -> Result<SaveActor, V3Error> {
    SaveActor::from_actor(who).ok_or_else(role_not_allowed_here)
}

/// The reversal named by the path, after checking it is this branch's.
async fn this_branchs(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    id: &str,
) -> Result<ReversalId, V3Error> {
    let id = Uuid::parse_str(id).map(ReversalId).map_err(|_| {
        V3Error::new(
            "validationFailed",
            Some("id"),
            "invalid reversal id",
            StatusCode::BAD_REQUEST,
        )
    })?;
    let owner: Option<String> = sqlx::query_scalar("SELECT branch_id FROM reversals WHERE id = $1")
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| server_error())?;
    match owner {
        None => Err(V3Error::new(
            "notFound",
            Some("id"),
            "no such reversal",
            StatusCode::NOT_FOUND,
        )),
        Some(owner) if owner != branch.as_str() => Err(out_of_scope()),
        Some(_) => Ok(id),
    }
}

pub struct ReversalPostCommand {
    pool: PgPool,
    service: PostReversal,
}

impl ReversalPostCommand {
    pub fn new(pool: PgPool, files: FileService) -> Self {
        Self {
            service: PostReversal::new(
                pool.clone(),
                files,
                Arc::new(LedgerCashPostings::new()),
                Arc::new(ExchangeEffects::new(Vec::new())),
            ),
            pool,
        }
    }
}

impl CommandAction for ReversalPostCommand {
    const NAME: &'static str = "reversalPost";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "reversalRequest");
    type Input = ReversalPostInput;
    type Output = ReversalPostedDto;
    const ROUTE: &'static str = "/api/reversals/{id}/post";
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
        let actor = save_actor(who)?;
        self.service
            .run_in(tx, id, &actor, Utc::now())
            .await
            .map_err(refusal)?;
        let Some(reversal) = PgReversalRepository::new(self.pool.clone())
            .find_in(tx, id)
            .await
            .map_err(|_| server_error())?
        else {
            return Err(server_error());
        };
        let document_number: String = sqlx::query_scalar(
            "SELECT r.number FROM receipts r JOIN reversals v ON v.receipt_id = r.id WHERE v.id = $1",
        )
        .bind(id.0)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| server_error())?;
        Ok(ReversalPostedDto {
            reversal: ReversalDto::from_domain(&reversal),
            document_number,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    let files = FileService::new(pool.clone(), state.settings.files.dir.clone());
    CommandEndpoint::register(cfg, Arc::new(ReversalPostCommand::new(pool, files)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ReversalPostInput::export_all(&cfg)?;
    ReversalPostedDto::export_all(&cfg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The TS spelled for `ReversalPostedDto.reversal` names exactly the keys `ReversalDto` serialises.
    #[test]
    fn the_reversal_dto_shape_is_the_one_ts_names() {
        let dto = ReversalDto {
            id: String::new(),
            status: String::new(),
            original_transaction_id: String::new(),
            reason: String::new(),
            amount_thb: String::new(),
            business_date: String::new(),
            approval_request_id: String::new(),
        };
        let json = serde_json::to_value(&dto).expect("serialise");
        let mut keys: Vec<&str> = json
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let ts = ReversalPostedDto::decl(&ts_rs::Config::new());
        let mut named: Vec<&str> = [
            "id",
            "status",
            "originalTransactionId",
            "reason",
            "amountThb",
            "businessDate",
            "approvalRequestId",
        ]
        .into_iter()
        .filter(|k| ts.contains(&format!("{k}: string")))
        .collect();
        named.sort_unstable();
        assert_eq!(keys, named, "ReversalDto's keys, each named in the TS: {ts}");
    }
}
