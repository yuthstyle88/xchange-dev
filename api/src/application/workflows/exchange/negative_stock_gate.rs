//! S2 #3405: `NegativeStockGate` (design `section-b-gaps.md` §S2, `ledger-inventory.md` §5.1).
//!
//! XC-BE-GAPS-A: the gate is keyed on the posting's source ([`NegativeStockObject`]), so an exchange's
//! completion (#3405), a reversal's post (row 2) and a transfer's dispatch (row 3, R1) share it.

use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_auth::domain::access::Actor;
use xc_cash::application::ledger_approval::negative_stock::NEGATIVE_STOCK;
use xc_cash::domain::posting::ApprovalRequestId;
use xc_cash::domain::shortfall::StockShortfall;
use xc_exchange::domain::reversal::reversal::ReversalId;
use xc_exchange::domain::transaction::transaction::TransactionId;

/// Outcome of handling a `stockInsufficient` refusal at exchange completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegativeStockOutcome {
    ApprovalRequired(ApprovalRequestId),
    Rejected(ApprovalRequestId),
}

/// The source a `NEGATIVE_STOCK` request is about: the `source_id` of the posting that went short, which
/// `LedgerCashPostings::allows_negative` compares with the request's `object_id` (§5.1). `[required]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegativeStockObject {
    /// An exchange's completion (`source_type` `EXCHANGE`).
    Transaction(TransactionId),
    /// A reversal's post (`source_type` `REVERSAL`, `reversal.md` §4.3).
    Reversal(ReversalId),
    /// A cash transfer's dispatch (`source_type` `TRANSFER`, `cash_transfers.id` as text).
    Transfer(String),
}

impl NegativeStockObject {
    /// The request's `object_id`: the source's id as text.
    pub fn object_id(&self) -> String {
        match self {
            Self::Transaction(id) => id.0.to_string(),
            Self::Reversal(id) => id.0.to_string(),
            Self::Transfer(id) => id.clone(),
        }
    }
}

pub struct NegativeStockGate;

impl NegativeStockGate {
    /// `[required]` (#3405 §7): the APPROVED `NEGATIVE_STOCK` request whose `object_id` is `object`'s.
    pub async fn approved_for_in(
        tx: &mut Transaction<'_, Postgres>,
        object: &NegativeStockObject,
    ) -> AppResult<Option<ApprovalRequestId>> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id::text FROM approval_requests \
             WHERE type = $1 AND object_id = $2 AND status = 'APPROVED' \
             ORDER BY requested_at DESC, id DESC \
             LIMIT 1",
        )
        .bind(NEGATIVE_STOCK)
        .bind(object.object_id())
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(id.map(ApprovalRequestId::from))
    }

    /// Opens or re-reads the `NEGATIVE_STOCK` approval request in its own transaction after the source's
    /// posting refuses with `stockInsufficient` (#3405 §7). `refusal` is that refusal's message: the
    /// shortfall it carries becomes the request's `proposed_payload` (`StockShortfall`, row 1).
    pub async fn open_after_refusal(
        pool: &PgPool,
        approvals: &ApprovalService,
        object: &NegativeStockObject,
        refusal: &str,
        actor: &Actor,
        key: &str,
    ) -> AppResult<NegativeStockOutcome> {
        let object_id = object.object_id();
        let payload = StockShortfall::from_refusal(refusal).payload();
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
