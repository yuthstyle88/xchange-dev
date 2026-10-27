//! `LedgerAdjustmentService` (design `docs/design/blueprint/ledger-inventory.md` §13.5 "Create" and
//! "Post", AD-5, AD-7): creates an adjustment as its `LEDGER_ADJUSTMENT` request and posts an approved
//! one, both inside the command seam's transaction under the adjustment's branch lock (L-5). The
//! commands write the audit rows (AD-10).
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): the types and signatures are final; the bodies are
//! LA-4's.

use std::sync::Arc;

use pos_error::AppResult;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::domain::request::RequestId;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::adjustment::AdjustmentInvalid;
use crate::domain::posting::{CashPostings, PostingId};
use crate::domain::shortfall::StockShortfall;

/// What `create_in` answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// `ADJ-<uuid v4>`.
    pub id: String,
    pub approval_request_id: RequestId,
}

/// One posted figure, read back from the posting (§13.5 step 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedLine {
    pub currency: CurrencyCode,
    /// QUANTITY: the signed quantity on the SAFE account's entry. COST: 0.
    pub quantity: Decimal,
    /// QUANTITY: the absolute `valuation_thb` of the SAFE account's entry (`"3250.00"` for USD −100 at
    /// 32.50, LA-T15). COST: the line's signed `costDeltaThb`.
    pub valuation_thb: Decimal,
}

/// What `post_in` answers. A repeat answers the same `posting_id` (§13.5 step 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub id: String,
    pub posting_id: PostingId,
    /// In the payload's line order.
    pub lines: Vec<PostedLine>,
}

/// A refusal the commands answer as a client error; anything else is an `AppError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdjustmentRefusal {
    /// `400 validationFailed` naming the field (§13.4). Also the port's own `validationFailed` on a
    /// post (no published BUY rate to cost stock with no history, LA-T23), on `lines`.
    Invalid(AdjustmentInvalid),
    /// `403 outOfScope`: the payload's `branch` is not the command's branch (create).
    OutOfScope,
    /// `404 notFound`: no `LEDGER_ADJUSTMENT` request with that object id in the caller's organisation,
    /// or its payload's branch is another branch (post).
    NotFound,
    /// `409 approvalRequired`: the request is PENDING (post).
    ApprovalRequired(RequestId),
    /// `409 staleVersion`: the request is REJECTED (post).
    Rejected(RequestId),
    /// `409 stockInsufficient`: the post would take the safe or the position below zero. No
    /// `NEGATIVE_STOCK` request is opened (AD-7).
    StockInsufficient(StockShortfall),
}

pub struct LedgerAdjustmentService {
    approvals: Arc<ApprovalService>,
    postings: Arc<dyn CashPostings>,
}

impl LedgerAdjustmentService {
    pub fn new(approvals: Arc<ApprovalService>, postings: Arc<dyn CashPostings>) -> Self {
        Self {
            approvals,
            postings,
        }
    }

    /// §13.5 "Create": reads `payload` as a new adjustment `ADJ-<uuid v4>`
    /// (`LedgerAdjustment::from_payload`), refuses a `branch` other than `branch` (`OutOfScope`),
    /// validates it against the `currencies` minor units, checks every `evidence.files` key is a
    /// `files` row of `branch` (`evidence.files`) and `exceptionId` a `cash_exceptions` row of `branch`
    /// (`exceptionId`), then opens the request:
    /// `ApprovalService::request(tx, LEDGER_ADJUSTMENT, id, <to_payload()>, who, "ledger-adjustment:" + id)`.
    /// Reads nothing of the ledger and writes no posting.
    pub async fn create_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        payload: &Value,
        who: &Actor,
    ) -> AppResult<Result<Created, AdjustmentRefusal>> {
        let _ = (&self.approvals, tx, branch, payload, who);
        todo!("LA-4")
    }

    /// §13.5 "Post": locks the request `FOR UPDATE` by `(type, object_id) = (LEDGER_ADJUSTMENT, id)` in
    /// `who`'s organisation; refuses as `AdjustmentRefusal` says; then QUANTITY posts
    /// `quantity_requests()` through `CashPostings::post_in(tx, .., None)` and COST posts
    /// `revalue_requests()` through one `CashPostings::revalue_in` call (never `post_in`). A repeat finds
    /// the posting by its source and writes nothing. Answers the figures read back from the posting.
    pub async fn post_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        id: &str,
        who: &Actor,
    ) -> AppResult<Result<Posted, AdjustmentRefusal>> {
        let _ = (&self.postings, tx, branch, id, who);
        todo!("LA-4")
    }
}
