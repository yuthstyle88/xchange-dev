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
use pos_error::{AppError, AppResult};
use serde_json::json;
use sqlx::{Acquire, PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::domain::reservation::stock_reservation::ReleaseReason;
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_exchange::domain::quote::binding_settings::BindingSettingsRepository;
use xc_exchange::domain::quote::quote::{Quote, QuoteStatus};
use xc_exchange::domain::quote::quote_type::QuoteType;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::settings::repository::SettingsRepository;
use xc_exchange::infrastructure::quote::binding_settings::PgBindingSettings;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::settings::repository::PgSettingsRepository;
use xc_kernel::money::{ForeignAmount, Money, Side};
use xc_verification::application::readiness::Readiness;

use super::binding_approval::{self, BINDING_QUOTE};
use super::quote::{QuoteRequest, QuoteService, QuoteTerms};
use crate::application::workflows::save_step::SaveActor;

/// `stockInsufficient` refusals keep B05's text; the codes below are this workflow's own.
/// The retry names a request that is not this proposal's, or not a `BINDING_QUOTE` request (Q22).
pub const STALE_PROPOSAL: &str = "staleVersion: the approval request names another binding quote";
/// The named request was REJECTED (Q22).
pub const APPROVAL_REJECTED: &str = "approvalRejected: the binding quote was rejected";
/// The named APPROVED request has already issued a binding quote (D-20, Q32): a request is consumed by the one
/// quote it issues (`quotes.approval_request_id`, UNIQUE).
pub const APPROVAL_USED: &str =
    "staleVersion: the approval request has already issued a binding quote";
/// The named APPROVED request approved a lower `settleThb` than the quote prices at now (D-15, Q36): nothing is
/// issued, and the request stays unused.
pub const APPROVAL_PRICE_MOVED: &str =
    "staleVersion: the price is above the amount the approval request approved";
/// AP-1 refused the request's idempotency key as reused with another payload (D-15, BQ-10).
pub const APPROVAL_REQUEST_CHANGED: &str =
    "staleVersion: the binding quote's approval request changed; ask again";

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
    /// Boxed: a quote is far larger than a request id (clippy `large_enum_variant`).
    Issued(Box<IssuedQuote>),
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
        let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
            .bind(req.quote.branch_id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?
            .ok_or_else(|| AppError::Domain("validationFailed: branch not found".to_owned()))?;

        let policy = PgBindingSettings::new().load_in(tx, org).await?;
        let ttl = policy.enabled_ttl()?;
        policy.check_channel(req.quote.channel)?;

        let verified_until = match req.quote.customer_id.as_deref() {
            Some(cid) => match Readiness::of(&self.pool, cid)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?
            {
                Readiness::Verified { until } => Some(until),
                _ => None,
            },
            None => None,
        };
        policy.check_customer(verified_until, now)?;

        let proposal_id = binding_approval::proposal_id(&req.quote, req.coupon_code.as_deref());
        let mut approved_request_id: Option<Uuid> = None;
        if let Some(ref req_id) = req.approval_request_id {
            let req_uuid =
                Uuid::parse_str(req_id).map_err(|_| AppError::Domain(STALE_PROPOSAL.to_owned()))?;
            let row: Option<(String, String, String)> = sqlx::query_as(
                "SELECT type, object_id, status FROM approval_requests WHERE id = $1",
            )
            .bind(req_uuid)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let Some((type_name, object_id, status)) = row else {
                return Err(AppError::Domain(STALE_PROPOSAL.to_owned()));
            };
            if type_name != BINDING_QUOTE || object_id != proposal_id {
                return Err(AppError::Domain(STALE_PROPOSAL.to_owned()));
            }
            match status.as_str() {
                "REJECTED" => return Err(AppError::Domain(APPROVAL_REJECTED.to_owned())),
                "PENDING" => {
                    return Ok(BindingIssueOutcome::ApprovalRequired(req_id.clone()));
                }
                "APPROVED" => {
                    let used: bool = sqlx::query_scalar(
                        "SELECT EXISTS (SELECT 1 FROM quotes WHERE approval_request_id = $1)",
                    )
                    .bind(req_uuid)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
                    if used {
                        return Err(AppError::Domain(APPROVAL_USED.into()));
                    }
                    approved_request_id = Some(req_uuid);
                }
                _ => return Err(AppError::Domain(STALE_PROPOSAL.to_owned())),
            }
        }

        let mut sp = (**tx).begin().await.map_err(AppError::Database)?;

        let quote = QuoteService::new(self.pool.clone())
            .price_in(
                &mut sp,
                req.quote.clone(),
                who,
                now,
                req.coupon_code.as_deref(),
                QuoteTerms::Binding { ttl },
            )
            .await?;

        let total_minor: i128 = quote.lines.iter().map(|l| l.settle_thb.minor_i128()).sum();
        let total = Money::try_from_minor_i128(total_minor)
            .map_err(|_| AppError::Domain("invalid settle amount".to_owned()))?;

        let settings = PgSettingsRepository::new(self.pool.clone());
        let limit = settings
            .approval_limit_in(&mut sp, org, &req.quote.branch_id)
            .await?;

        if approved_request_id.is_none() && policy.needs_approval(total, limit) {
            sp.rollback().await.map_err(AppError::Database)?;

            let payload = json!({
                "settleThb": total.to_string(),
                "approvalLimitThb": limit.map(|l| l.to_string()),
                "branch": req.quote.branch_id.as_str(),
            });
            let actor = who.access_actor_at(&org.to_string(), &req.quote.branch_id, now);
            let key = format!("bindingQuote:{proposal_id}");
            let mut own_tx = self.pool.begin().await.map_err(AppError::Database)?;

            return match self
                .approvals
                .request(
                    &mut own_tx,
                    BINDING_QUOTE,
                    &proposal_id,
                    &payload,
                    &actor,
                    &key,
                )
                .await
            {
                Ok(ar) if ar.status == "REJECTED" => {
                    Err(AppError::Domain(APPROVAL_REJECTED.to_owned()))
                }
                Ok(ar) => {
                    own_tx.commit().await.map_err(AppError::Database)?;
                    Ok(BindingIssueOutcome::ApprovalRequired(
                        ar.id.as_str().to_owned(),
                    ))
                }
                Err(ApprovalError::AlreadyPending(_)) => {
                    let pending_id: Option<String> = sqlx::query_scalar(
                        "SELECT id::text FROM approval_requests \
                         WHERE type = $1 AND object_id = $2 AND status = 'PENDING' \
                         ORDER BY requested_at DESC, id DESC LIMIT 1",
                    )
                    .bind(BINDING_QUOTE)
                    .bind(&proposal_id)
                    .fetch_optional(&mut *own_tx)
                    .await
                    .map_err(AppError::Database)?;
                    let id = pending_id.ok_or_else(|| {
                        AppError::Internal(format!(
                            "AlreadyPending for {BINDING_QUOTE} {proposal_id} but no PENDING row found"
                        ))
                    })?;
                    Ok(BindingIssueOutcome::ApprovalRequired(id))
                }
                Err(e) => Err(AppError::Internal(format!(
                    "failed to open {BINDING_QUOTE} approval request: {e}"
                ))),
            };
        }

        let reservations = PgStockReservations::new();
        let mut lines = Vec::with_capacity(quote.lines.len());
        for line in &quote.lines {
            let reserved_quantity = match line.shop_side {
                Side::Sell => {
                    reservations
                        .reserve_in(
                            &mut sp,
                            &ReserveLine {
                                quote_line_id: line.id,
                                denomination_id: None,
                            },
                            now,
                        )
                        .await?;
                    line.quantity
                }
                Side::Buy => ForeignAmount::ZERO,
            };
            lines.push(ReservedLine {
                quote_line_id: line.id,
                reserved_quantity,
            });
        }

        if let Some(replaces_id) = req.replaces {
            let quote_repo = PgQuoteRepository::new(self.pool.clone());
            let old = quote_repo
                .find_in(&mut sp, replaces_id)
                .await?
                .ok_or_else(|| {
                    AppError::Domain(format!(
                        "validationFailed:replacesQuoteId: quote {replaces_id} does not exist"
                    ))
                })?;
            if old.quote_type != QuoteType::Binding
                || old.status != QuoteStatus::Active
                || old.branch_id != req.quote.branch_id
            {
                return Err(AppError::Domain(format!(
                    "validationFailed:replacesQuoteId: quote {replaces_id} is not an active binding quote of this branch"
                )));
            }
            quote_repo.cancel_in(&mut sp, replaces_id).await?;
            reservations
                .release_for_quote_in(&mut sp, replaces_id, ReleaseReason::Cancelled)
                .await?;
        }

        if let Some(req_uuid) = approved_request_id {
            sqlx::query("UPDATE quotes SET approval_request_id = $1 WHERE id = $2")
                .bind(req_uuid)
                .bind(quote.id)
                .execute(&mut *sp)
                .await
                .map_err(|e| match e {
                    sqlx::Error::Database(ref db) if db.code().as_deref() == Some("23505") => {
                        AppError::Domain(APPROVAL_USED.into())
                    }
                    other => AppError::Database(other),
                })?;
        }

        sp.commit().await.map_err(AppError::Database)?;
        Ok(BindingIssueOutcome::Issued(Box::new(IssuedQuote {
            quote,
            lines,
        })))
    }
}
