//! S2 #3405: `NegativeStockGate` (design `section-b-gaps.md` §S2, `ledger-inventory.md` §5.1).

use pos_error::{AppError, AppResult};
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_auth::domain::access::Actor;
use xc_cash::application::ledger_approval::negative_stock::NEGATIVE_STOCK;
use xc_cash::domain::posting::ApprovalRequestId;
use xc_exchange::domain::transaction::transaction::TransactionId;

/// Outcome of handling a `stockInsufficient` refusal at exchange completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegativeStockOutcome {
    ApprovalRequired(ApprovalRequestId),
    Rejected(ApprovalRequestId),
}

pub struct NegativeStockGate;

impl NegativeStockGate {
    /// `[required]` (#3405 §7): the APPROVED `NEGATIVE_STOCK` request whose `object_id` is `transaction_id`.
    pub async fn approved_for_in(
        tx: &mut Transaction<'_, Postgres>,
        transaction_id: TransactionId,
    ) -> AppResult<Option<ApprovalRequestId>> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id::text FROM approval_requests \
             WHERE type = $1 AND object_id = $2 AND status = 'APPROVED' \
             ORDER BY requested_at DESC, id DESC \
             LIMIT 1",
        )
        .bind(NEGATIVE_STOCK)
        .bind(transaction_id.0.to_string())
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(id.map(ApprovalRequestId::from))
    }

    /// Opens or re-reads the `NEGATIVE_STOCK` approval request in its own transaction after
    /// `CompleteExchange::run_in` refuses with `stockInsufficient` (#3405 §7).
    pub async fn open_after_refusal(
        pool: &PgPool,
        approvals: &ApprovalService,
        transaction_id: TransactionId,
        actor: &Actor,
        key: &str,
    ) -> AppResult<NegativeStockOutcome> {
        let object_id = transaction_id.0.to_string();
        let payload = json!({ "shortfall": [] });
        let mut tx = pool.begin().await.map_err(AppError::Database)?;

        match approvals
            .request(&mut tx, NEGATIVE_STOCK, &object_id, &payload, actor, key)
            .await
        {
            Ok(req) if req.status == "REJECTED" => Ok(NegativeStockOutcome::Rejected(req.id)),
            Ok(req) => {
                tx.commit().await.map_err(AppError::Database)?;
                Ok(NegativeStockOutcome::ApprovalRequired(req.id))
            }
            Err(ApprovalError::AlreadyPending(_)) => {
                let pending_id: Option<String> = sqlx::query_scalar(
                    "SELECT id::text FROM approval_requests \
                     WHERE type = $1 AND object_id = $2 AND status = 'PENDING' \
                     ORDER BY requested_at DESC, id DESC LIMIT 1",
                )
                .bind(NEGATIVE_STOCK)
                .bind(&object_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::Database)?;
                let id = pending_id.ok_or_else(|| {
                    AppError::Internal(format!(
                        "AlreadyPending for {NEGATIVE_STOCK} {object_id} but no PENDING row found"
                    ))
                })?;
                Ok(NegativeStockOutcome::ApprovalRequired(
                    ApprovalRequestId::from(id),
                ))
            }
            Err(e) => Err(AppError::Internal(format!(
                "failed to open NEGATIVE_STOCK approval request: {e}"
            ))),
        }
    }
}
