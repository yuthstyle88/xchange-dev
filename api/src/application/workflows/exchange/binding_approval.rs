//! BQ-6: `BindingQuoteSubject` and `BindingQuoteApprovalPolicy` (design `docs/design/blueprint/binding-quote.md`
//! D-15, D-19; O-5 A): a binding quote whose total `settle_thb` exceeds its branch's limit waits for the
//! BRANCH_MANAGER of that branch before it is issued (T2 reused at issue).
//!
//! No quote exists while the request waits, so the request names the **proposal**: its `object_id` is
//! [`proposal_id`], a sha256 over the canonical issue request. The object never changes (a changed request is
//! another proposal), so `version_hash` is the `object_id` itself and `on_decided` writes nothing; the retry that
//! names the APPROVED request issues the quote (`binding_issue.rs`).

use std::sync::Arc;

use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, ApprovalDecider, Permission};
use xc_kernel::money::{Money, Side};

use super::quote::QuoteRequest;

type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// The approval type of a binding quote over its branch's limit (D-15).
pub const BINDING_QUOTE: &str = "BINDING_QUOTE";

/// The `object_id` of a `BINDING_QUOTE` request: lowercase sha256 hex over the canonical issue request — branch,
/// customer, channel, payment method, coupon code and every line (currency, side, quantity normalised, fee,
/// denomination group), in line order. The same request gives the same id; a change to any of those gives
/// another id (Q22: *"a changed request is `staleVersion`"*).
pub fn proposal_id(req: &QuoteRequest, coupon_code: Option<&str>) -> String {
    let mut h = Sha256::new();
    h.update(format!("branch|{}\n", req.branch_id.as_str()));
    h.update(format!(
        "customer|{}\n",
        req.customer_id.as_deref().unwrap_or("-")
    ));
    h.update(format!("channel|{}\n", req.channel.as_str()));
    h.update(format!(
        "payment|{}\n",
        req.payment_method.map_or("-", |m| m.as_str())
    ));
    h.update(format!("coupon|{}\n", coupon_code.unwrap_or("-")));
    for line in &req.lines {
        h.update(format!(
            "line|{}|{}|{}|{}|{}\n",
            line.currency.as_str(),
            match line.shop_side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            },
            line.quantity.value().normalize(),
            line.fee_thb
                .map_or_else(|| "-".to_owned(), |f| f.to_string()),
            line.denomination_group,
        ));
    }
    format!("{:x}", h.finalize())
}

/// D-20 (Q33–Q35): whether the quote's own approval stands for T2 at confirm. `true` only when quote `quote_id` is a
/// `BINDING` quote whose `quotes.approval_request_id` names an `APPROVED` `BINDING_QUOTE` request whose
/// `proposed_payload.settleThb` is at least `settle_thb`. Anything else (no request, another status or type, a smaller
/// approved amount, an indicative quote) is `false`, and confirm asks T2 as today. Reads in the caller's transaction.
pub async fn covers_in(
    tx: &mut Transaction<'_, Postgres>,
    quote_id: uuid::Uuid,
    settle_thb: Money,
) -> AppResult<bool> {
    let row: Option<Option<String>> = sqlx::query_scalar(
        "SELECT r.proposed_payload->>'settleThb' \
         FROM quotes q \
         JOIN approval_requests r ON r.id = q.approval_request_id \
         WHERE q.id = $1 AND q.type = 'BINDING' AND r.type = 'BINDING_QUOTE' AND r.status = 'APPROVED'",
    )
    .bind(quote_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(AppError::Database)?;

    Ok(row
        .flatten()
        .and_then(|s| Money::parse(&s).ok())
        .is_some_and(|approved| approved.minor_i128() >= settle_thb.minor_i128()))
}

pub struct BindingQuoteSubject;

pub struct BindingQuoteApprovalPolicy;

/// Registers `BINDING_QUOTE` with AP-1 (`bootstrap/approval_subjects/binding_quote.rs`).
pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    let _ = pool;
    registry.register(
        Arc::new(BindingQuoteSubject),
        Arc::new(BindingQuoteApprovalPolicy),
    );
}

impl ApprovalSubject for BindingQuoteSubject {
    fn type_name(&self) -> &'static str {
        BINDING_QUOTE
    }

    /// The proposal is immutable: its hash is its `object_id`.
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        let _ = tx;
        Box::pin(async move { Ok(object_id.to_owned()) })
    }

    /// Nothing to write: the quote is issued by the retry that names the APPROVED request (D-15).
    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        let _ = (tx, object_id, outcome);
        Box::pin(async move { Ok(()) })
    }
}

impl ApprovalPolicy for BindingQuoteApprovalPolicy {
    /// The BRANCH_MANAGER of the request's branch only, as T2 (`quote-transaction.md` §5.1): the holder of
    /// `Permission::ApproveExchangeTransaction` on a row covering `request.branch_id` (RBAC §11.9). Anyone else is
    /// `Refusal("outOfScope", …)`. The proposer is refused by AP-1 itself.
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        if ApprovalDecider::holds_on(
            who,
            Permission::ApproveExchangeTransaction,
            request.branch_id.as_deref(),
        ) {
            Ok(())
        } else {
            Err(Refusal::new(
                "outOfScope",
                "only the manager of the quote's branch decides it",
            ))
        }
    }

    fn policy_version(&self) -> String {
        "1".to_owned()
    }
}
