//! BQ-6: `BindingQuoteSubject` and `BindingQuoteApprovalPolicy` (design `docs/design/blueprint/binding-quote.md`
//! D-15, D-19; O-5 A): a binding quote whose total `settle_thb` exceeds its branch's limit waits for the
//! BRANCH_MANAGER of that branch before it is issued (T2 reused at issue).
//!
//! No quote exists while the request waits, so the request names the **proposal**: its `object_id` is
//! [`proposal_id`], a sha256 over the canonical issue request. The object never changes (a changed request is
//! another proposal), so `version_hash` is the `object_id` itself and `on_decided` writes nothing; the retry that
//! names the APPROVED request issues the quote (`binding_issue.rs`).

use std::sync::Arc;

use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::Actor;

use super::quote::QuoteRequest;

type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// The approval type of a binding quote over its branch's limit (D-15).
pub const BINDING_QUOTE: &str = "BINDING_QUOTE";

/// The `object_id` of a `BINDING_QUOTE` request: lowercase sha256 hex over the canonical issue request — branch,
/// customer, channel, payment method, coupon code and every line (currency, side, quantity normalised, fee,
/// denomination group), in line order. The same request gives the same id; a change to any of those gives
/// another id (Q22: *"a changed request is `staleVersion`"*).
pub fn proposal_id(req: &QuoteRequest, coupon_code: Option<&str>) -> String {
    let _ = (req, coupon_code);
    todo!("BQ-6")
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
        let _ = (tx, object_id);
        Box::pin(async move { todo!("BQ-6") })
    }

    /// Nothing to write: the quote is issued by the retry that names the APPROVED request (D-15).
    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        let _ = (tx, object_id, outcome);
        Box::pin(async move { todo!("BQ-6") })
    }
}

impl ApprovalPolicy for BindingQuoteApprovalPolicy {
    /// The BRANCH_MANAGER of the request's branch only, as T2 (`quote-transaction.md` §5.1): the holder of
    /// `Permission::ApproveExchangeTransaction` on a row covering `request.branch_id` (RBAC §11.9). Anyone else is
    /// `Refusal("outOfScope", …)`. The proposer is refused by AP-1 itself.
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        let _ = (who, request);
        todo!("BQ-6")
    }

    fn policy_version(&self) -> String {
        todo!("BQ-6")
    }
}
