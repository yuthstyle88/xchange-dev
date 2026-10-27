//! BQ-3: `IssueBindingQuote` (design `docs/design/blueprint/binding-quote.md` D-4, D-6, D-9, D-10, D-14, D-15,
//! D-16, D-17): the rate lock of Blueprint phase 6, issued in the caller's transaction.
//!
//! Order: the organisation's `BindingQuotePolicy` (off ⇒ refused, D-10) → channel (D-14) → customer readiness
//! (D-17) → `QuoteService::price_in(…, QuoteTerms::Binding { ttl })` → the branch limit (D-15: over it ⇒ an AP-1
//! `BINDING_QUOTE` request and **no quote, no reservation** left in the caller's transaction, unless the request
//! names an APPROVED request for this very proposal) → `StockReservations::reserve_in` for every SELL line (D-4:
//! any refusal refuses the whole quote) → `replaces`, if named, `CANCELLED` and its reservations released (D-6).
//! No payment row is written (D-9). Idempotency (D-7) is the command seam's `Idempotency-Key`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_exchange::domain::quote::quote::Quote;
use xc_kernel::money::ForeignAmount;

use super::quote::QuoteRequest;
use crate::application::workflows::save_step::SaveActor;

/// `stockInsufficient` refusals keep B05's text; the codes below are this workflow's own.
/// The retry names a request that is not this proposal's, or not a `BINDING_QUOTE` request (Q22).
pub const STALE_PROPOSAL: &str = "staleVersion: the approval request names another binding quote";
/// The named request was REJECTED (Q22).
pub const APPROVAL_REJECTED: &str = "approvalRejected: the binding quote was rejected";

/// One binding-quote issue.
#[derive(Debug, Clone)]
pub struct BindingIssueRequest {
    pub quote: QuoteRequest,
    pub coupon_code: Option<String>,
    /// The `BINDING_QUOTE` request this retry names (D-15); `None` on a first try.
    pub approval_request_id: Option<String>,
    /// The ACTIVE binding quote this one replaces (D-6): cancelled, its reservations released, in the same
    /// transaction. Must be an ACTIVE `BINDING` quote of the same branch; else `validationFailed` on `replaces`.
    pub replaces: Option<Uuid>,
}

/// What one quote line holds (D-12): `reserved_quantity` is the line's quantity for a SELL line, zero for BUY.
#[derive(Debug, Clone, PartialEq)]
pub struct ReservedLine {
    pub quote_line_id: Uuid,
    pub reserved_quantity: ForeignAmount,
}

#[derive(Debug, Clone)]
pub struct IssuedQuote {
    pub quote: Quote,
    /// One per quote line, in line order.
    pub lines: Vec<ReservedLine>,
}

#[derive(Debug, Clone)]
pub enum BindingIssueOutcome {
    Issued(IssuedQuote),
    /// Over the branch limit (D-15): the PENDING `BINDING_QUOTE` request's id (a new one, or the one already
    /// pending for this proposal). Nothing else is written in the caller's transaction.
    ApprovalRequired(String),
}

pub struct IssueBindingQuote {
    pool: PgPool,
    approvals: Arc<ApprovalService>,
}

impl IssueBindingQuote {
    pub fn new(pool: PgPool, approvals: Arc<ApprovalService>) -> Self {
        Self { pool, approvals }
    }

    pub async fn run_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        req: BindingIssueRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<BindingIssueOutcome> {
        let _ = (&self.pool, &self.approvals, tx, req, who, now);
        todo!("BQ-3")
    }
}
