//! RV-2 (#682): `RequestReversal`, design `reversal.md` §4.2.
//!
//! A reversal is the whole exchange (V-1): a COMPLETED, LIVE exchange of the caller's branch, inside its business
//! day in the branch timezone (R-1), with no live reversal yet (V-5). The row is inserted REQUESTED, AP-1's request
//! is opened for it (it reads the row's `version_hash`, RG6), and the row is tied to the request, all in the
//! caller's transaction. The original is never changed (V-2).

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_exchange::domain::reversal::repository::ReversalRepository;
use xc_exchange::domain::reversal::reversal::{NewReversal, Reversal, ReversalId};
use xc_exchange::domain::reversal::window::{ReversalWindow, Tz};
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_exchange::infrastructure::reversal::repository::PgReversalRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

use super::reversal_approval::TRANSACTION_REVERSAL;
use crate::application::workflows::save_step::SaveActor;

pub struct RequestReversalInput {
    pub branch: BranchId,
    pub original: TransactionId,
    pub reason: String,
    pub key: String,
}

pub struct RequestReversal {
    reversals: PgReversalRepository,
    approvals: Arc<ApprovalService>,
}

fn refuse(code: &str, why: impl std::fmt::Display) -> AppError {
    AppError::Domain(format!("{code}: {why}"))
}

impl RequestReversal {
    pub fn new(pool: PgPool, approvals: Arc<ApprovalService>) -> Self {
        Self {
            reversals: PgReversalRepository::new(pool),
            approvals,
        }
    }

    pub async fn run_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        input: RequestReversalInput,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<Reversal> {
        let chars = input.reason.trim().chars().count();
        if chars == 0 || input.reason.chars().count() > 500 {
            return Err(refuse(
                "validationFailed",
                "the reason is 1 to 500 characters",
            ));
        }

        // The original, at the caller's branch: an unknown exchange answers as another branch's does
        let Some(t) = sqlx::query(
            "SELECT t.branch_id, t.mode, t.status, t.completed_at, t.organization_id, b.timezone \
             FROM transactions t JOIN branches b ON b.id = t.branch_id WHERE t.id = $1",
        )
        .bind(input.original.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?
        else {
            return Err(refuse("outOfScope", "no such exchange at this branch"));
        };
        let branch: String = t.try_get("branch_id").map_err(AppError::Database)?;
        let mode: String = t.try_get("mode").map_err(AppError::Database)?;
        if branch != input.branch.as_str() || mode != "LIVE" {
            return Err(refuse("outOfScope", "no such LIVE exchange at this branch"));
        }
        let status: String = t.try_get("status").map_err(AppError::Database)?;
        let completed: Option<DateTime<Utc>> =
            t.try_get("completed_at").map_err(AppError::Database)?;
        let (Some(completed), "COMPLETED") = (completed, status.as_str()) else {
            return Err(refuse(
                "staleVersion",
                format!("a {status} exchange is not reversed"),
            ));
        };

        // R-1: inside the original's business day, in the branch timezone
        let zone: String = t.try_get("timezone").map_err(AppError::Database)?;
        let tz = Tz::from_str(&zone)
            .map_err(|_| AppError::Internal(format!("branch timezone {zone}")))?;
        let business_date = ReversalWindow::business_date_of(completed, tz);
        if !ReversalWindow::is_open(business_date, now, tz) {
            return Err(refuse(
                "reversalWindowClosed",
                format!("the business day {business_date} is over"),
            ));
        }

        // V-1: the whole settled THB of the exchange
        let amount: Option<String> = sqlx::query_scalar(
            "SELECT sum(settle_thb)::text FROM transaction_lines WHERE transaction_id = $1",
        )
        .bind(input.original.0)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        let amount_thb = amount
            .as_deref()
            .map(Money::parse)
            .transpose()
            .map_err(|e| AppError::Internal(format!("settled THB: {e}")))?
            .ok_or_else(|| AppError::Internal("an exchange without lines".into()))?;

        let id = ReversalId(Uuid::new_v4());
        let organization_id: Uuid = t.try_get("organization_id").map_err(AppError::Database)?;
        self.reversals
            .insert_in(
                tx,
                &NewReversal {
                    id,
                    organization_id,
                    branch_id: input.branch.clone(),
                    original_transaction_id: input.original,
                    reason: input.reason.clone(),
                    amount_thb,
                    business_date,
                    requested_by: who.id.clone(),
                    requested_at: now,
                },
            )
            .await?;

        // AP-1: the requester at the exchange's branch (the owner has none)
        let by = Actor::staff(who.id.clone(), who.role, input.branch.clone())
            .unwrap_or_else(|| Actor::owner(who.id.clone()));
        let request = self
            .approvals
            .request(
                tx,
                TRANSACTION_REVERSAL,
                &id.0.to_string(),
                &serde_json::json!({
                    "originalTransactionId": input.original.0.to_string(),
                    "reason": input.reason,
                    "amountThb": amount_thb.to_string(),
                }),
                &by,
                &input.key,
            )
            .await
            .map_err(|e| refuse(e.code(), e))?;
        let request_id = Uuid::from_str(request.id.as_str())
            .map_err(|_| AppError::Internal(format!("request id {}", request.id.as_str())))?;
        self.reversals
            .set_approval_request_in(tx, id, request_id)
            .await?;

        self.reversals
            .find_in(tx, id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("reversal {} reads back", id.0)))
    }
}
