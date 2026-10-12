//! RV-2 (#682): `ReversalSubject` and `ReversalPolicy` for AP-1, design `reversal.md` §5 (R-3: FINANCE approves).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::Utc;
use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role};
use xc_exchange::domain::reversal::repository::ReversalRepository;
use xc_exchange::domain::reversal::reversal::{ReversalId, ReversalStatus};
use xc_exchange::infrastructure::reversal::repository::PgReversalRepository;

/// AP-1's `approval_requests.type` of a reversal.
pub const TRANSACTION_REVERSAL: &str = "TRANSACTION_REVERSAL";

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

fn reversal_id(object_id: &str) -> AppResult<ReversalId> {
    Uuid::parse_str(object_id)
        .map(ReversalId)
        .map_err(|_| AppError::Internal(format!("reversal id {object_id}")))
}

pub struct ReversalSubject {
    reversals: PgReversalRepository,
}

impl ApprovalSubject for ReversalSubject {
    fn type_name(&self) -> &'static str {
        TRANSACTION_REVERSAL
    }

    /// Over the original, the amount, the reason and the status: a reversal changed after its request makes the
    /// decision stale.
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT original_transaction_id::text AS original, amount_thb::text AS amount, reason, status \
                 FROM reversals WHERE id = $1",
            )
            .bind(reversal_id(object_id)?.0)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            let get = |c: &str| row.try_get::<String, _>(c).map_err(AppError::Database);
            let state = format!(
                "{}|{}|{}|{}",
                get("original")?,
                get("amount")?,
                get("reason")?,
                get("status")?
            );
            Ok(format!("{:x}", Sha256::digest(state.as_bytes())))
        })
    }

    /// APPROVED ⇒ `APPROVED`, REJECTED ⇒ `REJECTED`, from REQUESTED.
    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let to = match outcome {
                Outcome::Approved => ReversalStatus::Approved,
                Outcome::Rejected => ReversalStatus::Rejected,
            };
            self.reversals
                .set_status_in(
                    tx,
                    reversal_id(object_id)?,
                    ReversalStatus::Requested,
                    to,
                    Utc::now(),
                )
                .await
        })
    }
}

/// R-3: FINANCE of the reversal's branch decides; nobody else, and never the requester.
pub struct ReversalPolicy;

impl ApprovalPolicy for ReversalPolicy {
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        let req_branch = request.branch_id.as_deref();
        let is_finance_of_branch = who.roles.iter().any(|r| {
            r.role == Role::Finance
                && (match (r.branch.as_ref(), req_branch) {
                    (Some(b), Some(rb)) => b.as_str() == rb,
                    _ => false,
                })
        });
        if is_finance_of_branch {
            Ok(())
        } else {
            Err(Refusal::new(
                "outOfScope",
                "FINANCE of the branch decides a reversal",
            ))
        }
    }

    fn allows_self_decision(&self, _who: &Actor) -> bool {
        false
    }

    fn policy_version(&self) -> String {
        "1".into()
    }
}

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    registry.register(
        Arc::new(ReversalSubject {
            reversals: PgReversalRepository::new(pool),
        }),
        Arc::new(ReversalPolicy),
    );
}
