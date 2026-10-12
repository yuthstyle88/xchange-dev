//! TX-4 #672: `ConfirmTransaction` and `ConfirmOutcome` (design §4.4, §7): a DRAFT whose quote is
//! still valid runs every save step, accepts its quote, and is CONFIRMED, or waits for the branch
//! manager when it settles above the branch's limit (T2). Waiting is an outcome, not an error.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::domain::request::RequestId;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::v3_roles::actor_from_v3;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::money::Money;

use super::approval::EXCHANGE_TRANSACTION;
use super::create::check_valid_in;
use crate::application::workflows::save_step::{
    BookingClaim, SaveActor, SaveContext, SaveExtras, SaveStep,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    Confirmed,
    PendingApproval(RequestId),
}

pub struct ConfirmTransaction {
    store: PgTransactionStore,
    quotes: PgQuoteRepository,
    settings: PgSettingsRepository,
    approvals: Arc<ApprovalService>,
    steps: Vec<Arc<dyn SaveStep>>,
    audit: PgAuditLog,
}

/// G3: the requester AP-1 is told about: the saver at the **transaction's** branch (the owner has
/// no branch).
fn actor_of(who: &SaveActor, t: &ExchangeTransaction) -> AppResult<xc_auth::domain::access::Actor> {
    let v3 = if who.role == Role::Owner {
        Actor::owner(who.id.clone())
    } else {
        Actor::staff(who.id.clone(), who.role, t.branch_id.clone()).ok_or_else(|| {
            AppError::Internal(format!("{} is not a staff role", who.role.as_str()))
        })?
    };
    Ok(actor_from_v3(
        &v3,
        &t.organization_id.to_string(),
        Utc::now(),
    ))
}

/// An approval core refusal as the exchange's own refusal: its code first.
fn approval_refusal(e: ApprovalError) -> AppError {
    match e {
        ApprovalError::Database(m) | ApprovalError::Internal(m) => AppError::Internal(m),
        other => AppError::Domain(format!("{}: {other}", other.code())),
    }
}

/// The THB that changes hands, summed over the lines (§5.2).
fn settle_of(t: &ExchangeTransaction) -> AppResult<Money> {
    let minor: i128 = t.lines.iter().map(|l| l.settle_thb.minor_i128()).sum();
    Money::try_from_minor_i128(minor).map_err(|e| AppError::Internal(format!("settle amount: {e}")))
}

impl ConfirmTransaction {
    pub fn new(
        pool: PgPool,
        approvals: Arc<ApprovalService>,
        steps: Vec<Arc<dyn SaveStep>>,
    ) -> Self {
        Self {
            store: PgTransactionStore::new(pool.clone()),
            quotes: PgQuoteRepository::new(pool.clone()),
            settings: PgSettingsRepository::new(pool.clone()),
            approvals,
            steps,
            audit: PgAuditLog::new(pool),
        }
    }

    /// §4.4 confirm steps 1–5. Every refusal comes before the first write.
    pub async fn run_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        who: &SaveActor,
        extras: &SaveExtras,
        now: DateTime<Utc>,
    ) -> AppResult<ConfirmOutcome> {
        // 1. DRAFT only
        let Some(t) = self.store.find_in(tx, id).await? else {
            return Err(AppError::Domain(format!(
                "staleVersion: transaction {} does not exist",
                id.0
            )));
        };
        if t.status != TransactionStatus::Draft {
            return Err(AppError::Domain(format!(
                "staleVersion: a {} exchange is not confirmed",
                t.status.as_str()
            )));
        }

        // 2. The quote is still valid (D-5)
        let quote_id = t.quote_id.ok_or_else(|| {
            AppError::Domain(format!("staleVersion: transaction {} has no quote", id.0))
        })?;
        let Some(q) = self.quotes.find_in(tx, quote_id).await? else {
            return Err(AppError::Domain(format!(
                "staleVersion: quote {quote_id} does not exist"
            )));
        };
        check_valid_in(tx, &q, now).await?;

        // 3. Every registered step's before_in, unchanged (D-7), on the form of the stored exchange
        let form = TransactionForm::from_transaction(&t);
        let booking = t.booking_id.clone().map(|booking_id| BookingClaim {
            booking_id,
            customer_confirmed: true,
        });
        let ctx = SaveContext {
            branch: &t.branch_id,
            form: &form,
            extras,
            booking: booking.as_ref(),
            actor: who,
            // CA-7 (#623): no Blueprint-session teller here. Confirm knows only the `SaveActor`, and
            // the exchange's shift is the branch's v2 shift create took (as every save but
            // `save_as_teller` until CA-10 #626).
            teller: None,
            now,
            prior: None,
        };
        for step in &self.steps {
            step.before_in(tx, &ctx).await?;
        }

        // 4. The quote is accepted, bound to this exchange
        self.quotes.accept_in(tx, quote_id, id.0).await?;

        // 5. Above the branch's limit ⇒ an approval request and PENDING_APPROVAL
        let settle = settle_of(&t)?;
        let limit = self
            .settings
            .approval_limit_in(tx, t.organization_id, &t.branch_id)
            .await?;
        if let Some(limit) = limit
            && settle.minor_i128() > limit.minor_i128()
        {
            let request = self
                .approvals
                .request(
                    tx,
                    EXCHANGE_TRANSACTION,
                    &id.0.to_string(),
                    &json!({
                        "transactionId": id.0.to_string(),
                        "settleThb": settle.to_string(),
                        "approvalLimitThb": limit.to_string(),
                    }),
                    &actor_of(who, &t)?,
                    &format!("confirm:{}", id.0),
                )
                .await
                .map_err(approval_refusal)?;
            let request_uuid = Uuid::parse_str(request.id.as_str()).map_err(|_| {
                AppError::Internal(format!("request id {} is not a uuid", request.id.as_str()))
            })?;
            self.store
                .set_status_in(
                    tx,
                    id,
                    StatusMove {
                        from: TransactionStatus::Draft,
                        to: TransactionStatus::PendingApproval,
                        reason: None,
                        approval_request_id: Some(request_uuid),
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
                        action: "transaction.pendingApproval".to_string(),
                        object_ref: id.0.to_string(),
                        branch_id: Some(t.branch_id.as_str().to_string()),
                        before: Some(json!({ "status": "DRAFT" })),
                        after: Some(json!({
                            "status": "PENDING_APPROVAL",
                            "quoteId": quote_id.to_string(),
                            "approvalRequestId": request.id.as_str(),
                        })),
                    },
                )
                .await?;
            return Ok(ConfirmOutcome::PendingApproval(request.id));
        }
        self.store
            .set_status_in(
                tx,
                id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Confirmed,
                    reason: None,
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
                    action: "transaction.confirmed".to_string(),
                    object_ref: id.0.to_string(),
                    branch_id: Some(t.branch_id.as_str().to_string()),
                    before: Some(json!({ "status": "DRAFT" })),
                    after: Some(json!({
                        "status": "CONFIRMED",
                        "quoteId": quote_id.to_string(),
                    })),
                },
            )
            .await?;
        Ok(ConfirmOutcome::Confirmed)
    }
}
