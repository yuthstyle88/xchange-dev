//! TX-4 #672: `ExchangeTransactionSubject` and `ExchangeApprovalPolicy` (design §5.1): an exchange
//! above its branch's limit waits for the BRANCH_MANAGER of that branch (T2).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role};
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{TransactionId, TransactionStatus};
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::money::Side;

type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// The approval type of an exchange (§5.1).
pub const EXCHANGE_TRANSACTION: &str = "EXCHANGE_TRANSACTION";

pub struct ExchangeTransactionSubject {
    store: PgTransactionStore,
    quotes: PgQuoteRepository,
}

impl ExchangeTransactionSubject {
    pub fn new(pool: PgPool) -> Self {
        Self {
            store: PgTransactionStore::new(pool.clone()),
            quotes: PgQuoteRepository::new(pool),
        }
    }
}

pub struct ExchangeApprovalPolicy;

/// Registers `EXCHANGE_TRANSACTION` with AP-1 (`bootstrap/approval_subjects/exchange_transaction.rs`).
pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    registry.register(
        Arc::new(ExchangeTransactionSubject::new(pool)),
        Arc::new(ExchangeApprovalPolicy),
    );
}

/// The exchange an approval request's `object_id` names.
fn tid(object_id: &str) -> AppResult<TransactionId> {
    Uuid::parse_str(object_id).map(TransactionId).map_err(|_| {
        AppError::Internal(format!("approval object {object_id} is not an exchange id"))
    })
}

impl ApprovalSubject for ExchangeTransactionSubject {
    fn type_name(&self) -> &'static str {
        EXCHANGE_TRANSACTION
    }

    /// sha256 over the lines (currency, side, quantity, rate, settle), the payment (method,
    /// amount) and the customer: any change after the request makes the decision stale.
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let id = tid(object_id)?;
            let t = self
                .store
                .find_in(tx, id)
                .await?
                .ok_or_else(|| AppError::NotFound {
                    entity: "ExchangeTransaction",
                    id: object_id.to_owned(),
                })?;
            let mut h = Sha256::new();
            for line in &t.lines {
                h.update(format!(
                    "line|{}|{}|{}|{}|{}\n",
                    line.currency.as_str(),
                    match line.shop_side {
                        Side::Buy => "BUY",
                        Side::Sell => "SELL",
                    },
                    line.quantity.value().normalize(),
                    line.rate_snapshot.value().normalize(),
                    line.settle_thb,
                ));
            }
            h.update(format!(
                "payment|{}|{}\n",
                t.payment.method.as_str(),
                t.payment.amount_thb
            ));
            h.update(format!("customer|{}\n", t.customer_id));
            Ok(format!("{:x}", h.finalize()))
        })
    }

    /// APPROVED ⇒ `CONFIRMED`; REJECTED ⇒ `CANCELLED` with the reason of the decision `decide` has
    /// just written in this transaction (`on_decided` is not given it), and its quote `CANCELLED`.
    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let id = tid(object_id)?;
            let (reason, at): (String, DateTime<Utc>) = sqlx::query_as(
                "SELECT d.reason, d.at FROM approval_decisions d \
                 JOIN transactions t ON t.approval_request_id = d.request_id \
                 WHERE t.id = $1",
            )
            .bind(id.0)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            let r#move = match outcome {
                Outcome::Approved => StatusMove {
                    from: TransactionStatus::PendingApproval,
                    to: TransactionStatus::Confirmed,
                    reason: None,
                    approval_request_id: None,
                },
                Outcome::Rejected => StatusMove {
                    from: TransactionStatus::PendingApproval,
                    to: TransactionStatus::Cancelled,
                    reason: Some(reason),
                    approval_request_id: None,
                },
            };
            self.store.set_status_in(tx, id, r#move, at).await?;
            if matches!(outcome, Outcome::Rejected) {
                let t = self
                    .store
                    .find_in(tx, id)
                    .await?
                    .ok_or_else(|| AppError::NotFound {
                        entity: "ExchangeTransaction",
                        id: object_id.to_owned(),
                    })?;
                if let Some(q) = t.quote_id {
                    self.quotes.cancel_in(tx, q).await?;
                }
            }
            Ok(())
        })
    }
}

impl ApprovalPolicy for ExchangeApprovalPolicy {
    /// BRANCH_MANAGER of the exchange's branch only (§5.1). The EXECUTIVE is not named by T2.
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        let req_branch = request.branch_id.as_deref();
        let is_bm_of_branch = who.roles.iter().any(|r| {
            r.role == Role::BranchManager
                && (match (r.branch.as_ref(), req_branch) {
                    (Some(b), Some(rb)) => b.as_str() == rb,
                    _ => false,
                })
        });
        if is_bm_of_branch {
            Ok(())
        } else {
            Err(Refusal::new(
                "outOfScope",
                "only the manager of the exchange's branch decides it",
            ))
        }
    }

    fn policy_version(&self) -> String {
        "1".to_owned()
    }
}
