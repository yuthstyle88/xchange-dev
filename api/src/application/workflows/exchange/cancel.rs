//! TX-4 #672: `CancelTransaction` (design §4.5 cancel, §7): a DRAFT or CONFIRMED exchange is
//! cancelled with its reason, and its quote with it. Nothing was posted before COMPLETED, so
//! nothing is undone. A PENDING_APPROVAL one ends by the manager's decision only.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde_json::json;
use sqlx::PgPool;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;

use crate::application::workflows::save_step::SaveActor;

pub struct CancelTransaction {
    store: PgTransactionStore,
    quotes: PgQuoteRepository,
    audit: PgAuditLog,
}

impl CancelTransaction {
    pub fn new(pool: PgPool) -> Self {
        Self {
            store: PgTransactionStore::new(pool.clone()),
            quotes: PgQuoteRepository::new(pool.clone()),
            audit: PgAuditLog::new(pool),
        }
    }

    pub async fn run_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        reason: &str,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(AppError::Domain(
                "validationFailed: a cancel needs a reason".to_owned(),
            ));
        }
        let Some(t) = self.store.find_in(tx, id).await? else {
            return Err(AppError::Domain(format!(
                "staleVersion: transaction {} does not exist",
                id.0
            )));
        };
        if !matches!(
            t.status,
            TransactionStatus::Draft | TransactionStatus::Confirmed
        ) {
            return Err(AppError::Domain(format!(
                "staleVersion: a {} exchange is not cancelled",
                t.status.as_str()
            )));
        }
        let before_status = t.status.as_str();
        self.store
            .set_status_in(
                tx,
                id,
                StatusMove {
                    from: t.status,
                    to: TransactionStatus::Cancelled,
                    reason: Some(reason.to_owned()),
                    approval_request_id: None,
                },
                now,
            )
            .await?;
        self.audit
            .append(
                tx,
                &AuditEvent {
                    actor_id: who.id.clone(),
                    actor_role: who.role.as_str().to_string(),
                    scope: "branch".to_string(),
                    action: "transaction.cancelled".to_string(),
                    object_ref: id.0.to_string(),
                    branch_id: Some(t.branch_id.as_str().to_string()),
                    before: Some(json!({ "status": before_status })),
                    after: Some(json!({
                        "status": "CANCELLED",
                        "reason": reason,
                    })),
                },
            )
            .await?;
        if let Some(quote) = t.quote_id {
            self.quotes.cancel_in(tx, quote).await?;
        }
        self.store
            .find_in(tx, id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("transaction {} vanished", id.0)))
    }
}
