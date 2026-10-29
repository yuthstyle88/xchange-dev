//! `LedgerAdjustmentService` (design `docs/design/blueprint/ledger-inventory.md` §13.5 "Create" and
//! "Post", AD-5, AD-7): creates an adjustment as its `LEDGER_ADJUSTMENT` request and posts an approved
//! one, both inside the command seam's transaction under the adjustment's branch lock (L-5). The
//! commands write the audit rows (AD-10).
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): the types and signatures are final; the bodies are
//! LA-4's.

use std::collections::HashMap;
use std::sync::Arc;

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::domain::request::RequestId;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MinorUnit};

use crate::domain::adjustment::{
    AdjustmentInvalid, AdjustmentKind, ID_PREFIX, LEDGER_ADJUSTMENT, LedgerAdjustment,
};
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
        let id = format!("{ID_PREFIX}{}", Uuid::new_v4());
        let a = match LedgerAdjustment::from_payload(&id, payload) {
            Ok(a) => a,
            Err(e) => return Ok(Err(AdjustmentRefusal::Invalid(e))),
        };
        if a.branch != *branch {
            return Ok(Err(AdjustmentRefusal::OutOfScope));
        }

        let codes: Vec<String> = match &a.kind {
            AdjustmentKind::Quantity { lines, .. } => lines
                .iter()
                .map(|l| l.currency.as_str().to_owned())
                .collect(),
            AdjustmentKind::Cost { lines } => lines
                .iter()
                .map(|l| l.currency.as_str().to_owned())
                .collect(),
        };
        let mut units = HashMap::with_capacity(codes.len());
        if !codes.is_empty() {
            let rows: Vec<(String, i16)> =
                sqlx::query_as("SELECT code, minor_unit FROM currencies WHERE code = ANY($1)")
                    .bind(&codes)
                    .fetch_all(&mut **tx)
                    .await?;
            for (code, minor) in rows {
                let c = CurrencyCode::parse(code.trim())
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                let m = u8::try_from(minor)
                    .ok()
                    .and_then(|v| MinorUnit::new(v).ok())
                    .ok_or_else(|| AppError::Internal(format!("invalid minor_unit {minor}")))?;
                units.insert(c, m);
            }
        }
        if let Err(e) = a.validate(&units) {
            return Ok(Err(AdjustmentRefusal::Invalid(e)));
        }

        for key in &a.evidence.files {
            let exists: Option<i32> =
                sqlx::query_scalar("SELECT 1 FROM files WHERE key = $1 AND branch_id = $2")
                    .bind(key)
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await?;
            if exists.is_none() {
                return Ok(Err(AdjustmentRefusal::Invalid(AdjustmentInvalid::new(
                    "evidence.files",
                    "unknown or out-of-branch evidence file",
                ))));
            }
        }

        if let Some(ex_id) = &a.exception_id {
            let exists: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM cash_exceptions WHERE id = $1 AND branch_id = $2",
            )
            .bind(ex_id)
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await?;
            if exists.is_none() {
                return Ok(Err(AdjustmentRefusal::Invalid(AdjustmentInvalid::new(
                    "exceptionId",
                    "unknown or out-of-branch exception",
                ))));
            }
        }

        let req = self
            .approvals
            .request(
                tx,
                LEDGER_ADJUSTMENT,
                &id,
                Some(branch),
                &a.to_payload(),
                who,
                &format!("ledger-adjustment:{id}"),
            )
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

        Ok(Ok(Created {
            id,
            approval_request_id: req.id,
        }))
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
        let row: Option<(Uuid, String, Value)> = sqlx::query_as(
            "SELECT id, status, proposed_payload FROM approval_requests \
             WHERE type = 'LEDGER_ADJUSTMENT' AND object_id = $1 AND organization_id = $2::uuid \
             FOR UPDATE",
        )
        .bind(id)
        .bind(&who.organization_id)
        .fetch_optional(&mut **tx)
        .await?;
        let Some((req_id, status, proposed_payload)) = row else {
            return Ok(Err(AdjustmentRefusal::NotFound));
        };
        if proposed_payload.get("branch").and_then(Value::as_str) != Some(branch.as_str()) {
            return Ok(Err(AdjustmentRefusal::NotFound));
        }

        let req_id = RequestId::new(req_id.to_string());
        match status.as_str() {
            "PENDING" => return Ok(Err(AdjustmentRefusal::ApprovalRequired(req_id))),
            "REJECTED" => return Ok(Err(AdjustmentRefusal::Rejected(req_id))),
            "APPROVED" => {}
            other => {
                return Err(AppError::Internal(format!(
                    "unexpected LEDGER_ADJUSTMENT status: {other}"
                )));
            }
        }

        let a = LedgerAdjustment::from_payload(id, &proposed_payload).map_err(|e| {
            AppError::Internal(format!("invalid stored LEDGER_ADJUSTMENT: {}", e.message))
        })?;

        let post_res = match &a.kind {
            AdjustmentKind::Quantity { .. } => self
                .postings
                .post_in(tx, &a.quantity_requests(), None)
                .await
                .map(|r| r.posting_id),
            AdjustmentKind::Cost { .. } => {
                self.postings.revalue_in(tx, &a.revalue_requests()).await
            }
        };
        let posting_id = match post_res {
            Ok(pid) => pid,
            Err(AppError::Domain(m)) if m.starts_with("stockInsufficient") => {
                return Ok(Err(AdjustmentRefusal::StockInsufficient(
                    StockShortfall::from_refusal(&m),
                )));
            }
            Err(AppError::Domain(m)) if m.starts_with("validationFailed") => {
                return Ok(Err(AdjustmentRefusal::Invalid(AdjustmentInvalid::new(
                    "lines", m,
                ))));
            }
            Err(e) => return Err(e),
        };

        let lines = match &a.kind {
            AdjustmentKind::Quantity { lines, .. } => {
                let rows: Vec<(String, Decimal, Decimal)> = sqlx::query_as(
                    "SELECT trim(e.currency), e.quantity_signed, e.valuation_thb \
                     FROM ledger_entries e \
                     JOIN cash_accounts a ON a.id = e.account_id \
                     WHERE e.posting_id = $1 AND a.kind = 'SAFE'",
                )
                .bind(posting_id.0)
                .fetch_all(&mut **tx)
                .await?;
                let mut safe_entries = HashMap::with_capacity(rows.len());
                for (code, qty, val) in rows {
                    let c = CurrencyCode::parse(&code)
                        .map_err(|e| AppError::Internal(e.to_string()))?;
                    safe_entries.insert(c, (qty, val));
                }
                let mut out = Vec::with_capacity(lines.len());
                for line in lines {
                    let &(qty, val) = safe_entries.get(&line.currency).ok_or_else(|| {
                        AppError::Internal(format!(
                            "missing SAFE entry for {} on posting {}",
                            line.currency, posting_id.0
                        ))
                    })?;
                    out.push(PostedLine {
                        currency: line.currency,
                        quantity: qty.normalize(),
                        valuation_thb: val.abs(),
                    });
                }
                out
            }
            AdjustmentKind::Cost { lines } => lines
                .iter()
                .map(|l| PostedLine {
                    currency: l.currency,
                    quantity: Decimal::ZERO,
                    valuation_thb: l.cost_delta_thb,
                })
                .collect(),
        };

        Ok(Ok(Posted {
            id: a.id,
            posting_id,
            lines,
        }))
    }
}
