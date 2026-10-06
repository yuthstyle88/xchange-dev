//! The `adminAbandonRequest` workflow.
//!
//! A screen that got no answer to a write asks what became of its request id.
//! The id counts as applied when a branch was created with it, or when in this
//! branch a rate, an opened shift, a closed shift or a stock movement carries
//! it. Otherwise it is written to `abandoned_requests`, and from then on every
//! write of this branch refuses it (the services on main read that table).
//!
//! The check and the write happen inside one `BranchTx::run` of the branch, the
//! same lock the rate and shift writes take, so a request is never both applied
//! and given up.

use std::future::Future;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use serde_json::Value;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::domain::branch::repositories::branch_repository::BranchRepository;
use xc_rate::domain::rate::repositories::rate_repository::RateRepository;
use xc_rate::domain::validation::{BAD_TEXT, NO_SUCH_BRANCH, validate_request_id};
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;
use xc_rate::infrastructure::rate::rate_repository::PgRateRepository;
use xc_shift::domain::shift::repositories::shift_repository::ShiftRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::infrastructure::compat::abandon_request_repository::PgAbandonedRequests;

/// What became of a request id the screen gave up waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestResolution {
    /// The request had been applied; nothing is written.
    Applied,
    /// The request was never applied and never will be.
    Cancelled,
}

impl RequestResolution {
    /// The value of `requestResolution` in the answer.
    pub fn as_str(self) -> &'static str {
        match self {
            RequestResolution::Applied => "APPLIED",
            RequestResolution::Cancelled => "CANCELLED",
        }
    }
}

/// The write side of `abandoned_requests`. Reading it is the business of the
/// repositories of `xc-rate` and `xc-shift`.
pub trait AbandonedRequests: Send + Sync + 'static {
    /// Records that `request_id` was given up in `branch`. A row already there
    /// is left as it is: the first `at` and `actor` stay.
    fn insert_if_absent<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
        at: DateTime<Utc>,
        actor: &'a str,
    ) -> impl Future<Output = AppResult<()>> + Send + 'a;
}

/// The use case behind the action `adminAbandonRequest`.
#[derive(Clone)]
pub struct AbandonRequest {
    branch_tx: BranchTx,
    branches: PgBranchRepository,
    rates: PgRateRepository,
    shifts: Arc<PgShiftRepository>,
    abandoned: PgAbandonedRequests,
}

/// `payload.requestId` as v2 reads a text: text as it is, a number as its
/// digits, nothing as empty; then trimmed, 1 to 60 units, and of the shape of a
/// request id.
fn read_request_id(payload: &Value) -> AppResult<String> {
    let raw = match payload.get("requestId") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(_) => return Err(AppError::Domain(BAD_TEXT.to_string())),
    };
    Ok(validate_request_id(&raw)?.to_string())
}

fn cut_to_millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .unwrap_or(t)
}

impl AbandonRequest {
    pub fn new(branch_tx: BranchTx) -> Self {
        let pool = branch_tx.pool().clone();
        Self {
            branches: PgBranchRepository::new(pool.clone()),
            rates: PgRateRepository::new(pool.clone()),
            shifts: Arc::new(PgShiftRepository::new(pool)),
            abandoned: PgAbandonedRequests,
            branch_tx,
        }
    }

    /// Answers what became of `payload.requestId` in `branch`, giving it up
    /// when it was never applied.
    ///
    /// Refusals, in this order: the branch does not exist →
    /// `ไม่พบสาขาที่เลือก กรุณารีเฟรช`; no request id, or longer than 60 →
    /// `ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด`; not `[A-Za-z0-9-]{16,60}` →
    /// `รหัสคำขอไม่ถูกต้อง`.
    pub async fn abandon(
        &self,
        branch: &BranchId,
        payload: &Value,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<RequestResolution> {
        if !self.branches.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_string()));
        }
        let request_id = read_request_id(payload)?;

        let this = self.clone();
        let b = branch.clone();
        let actor = actor.to_owned();
        let at = cut_to_millis(now);
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    if this.is_applied(tx, &b, &request_id).await? {
                        return Ok(RequestResolution::Applied);
                    }
                    this.abandoned
                        .insert_if_absent(tx, &b, &request_id, at, &actor)
                        .await?;
                    Ok(RequestResolution::Cancelled)
                })
            })
            .await
    }

    /// Whether a write was made with `request_id`: a branch anywhere, or in
    /// `branch` a rate, a shift (opening or closing) or a stock movement.
    async fn is_applied(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        request_id: &str,
    ) -> AppResult<bool> {
        if self
            .branches
            .find_by_request(tx, request_id)
            .await?
            .is_some()
        {
            return Ok(true);
        }
        if self
            .rates
            .find_by_request_tx(tx, branch.as_str(), request_id)
            .await?
            .is_some()
        {
            return Ok(true);
        }
        if self
            .shifts
            .find_shift_by_open_request_id(tx, branch, request_id)
            .await?
            .is_some()
        {
            return Ok(true);
        }
        if self
            .shifts
            .find_shift_by_close_request_id(tx, branch, request_id)
            .await?
            .is_some()
        {
            return Ok(true);
        }
        Ok(self
            .shifts
            .find_movement_by_request_id(tx, branch, request_id)
            .await?
            .is_some())
    }
}
