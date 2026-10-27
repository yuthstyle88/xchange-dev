//! MAIN is the base; branches follow it in basis points (pricing-tiers.md §15.4; `H:` §5, §7).
//!
//! SKELETON (XC-V9-PRICING): `BranchPush::push`, `PushKind::row_source` and `BranchPushState::as_str` are PV-d's.
//! Types and signatures stay.

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;
use xc_auth::domain::access::Actor;

use crate::application::pricing::tier_publish::TierSetPublisher;
use crate::domain::pricing::branch_policy::BranchPolicyStore;
use crate::domain::pricing::tier_set::{PricingTierSet, TierSetSource, TierSetStore};

/// Which branches a push reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushKind {
    /// An approve at MAIN: every other active branch that is not `own` for the currency.
    Manual,
    /// The Follow publisher: only branches whose currency row has `follow = true` and not `own` (`H:` §7).
    Follow,
}

impl PushKind {
    /// The branch row's source: `Manual` ⇒ `MainPush`; `Follow` ⇒ `Follow` (§15.5).
    pub fn row_source(self) -> TierSetSource {
        todo!("PV-d: PushKind::row_source")
    }
}

/// One branch's outcome (§15.8a `branches[].state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchPushState {
    Published,
    SkippedOwn,
    Failed,
    /// A row with the same `request_id` already exists (a retry).
    AlreadyDone,
}

impl BranchPushState {
    /// `"PUBLISHED"` · `"SKIPPED_OWN"` · `"FAILED"` · `"ALREADY_DONE"`.
    pub fn as_str(self) -> &'static str {
        todo!("PV-d: BranchPushState::as_str")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchResult {
    pub branch: String,
    pub state: BranchPushState,
    pub tier_set_id: Option<Uuid>,
    /// The v3 error code when `Failed` (`validationFailed` for a `TierError`, else the mapped `RateEngineError`).
    pub error: Option<String>,
}

pub struct BranchPush {
    pool: PgPool,
    publisher: Arc<TierSetPublisher>,
    tier_sets: Arc<dyn TierSetStore>,
    policies: Arc<dyn BranchPolicyStore>,
}

impl BranchPush {
    pub fn new(
        pool: PgPool,
        publisher: Arc<TierSetPublisher>,
        tier_sets: Arc<dyn TierSetStore>,
        policies: Arc<dyn BranchPolicyStore>,
    ) -> Self {
        Self {
            pool,
            publisher,
            tier_sets,
            policies,
        }
    }

    /// Runs after MAIN's own publication of `base` committed. For each active branch other than MAIN, in branch id
    /// order, **each in its own transaction from the pool**: skip (`SkippedOwn`) when the branch is `own` for the
    /// currency (and, for `Follow`, when it is not enrolled — those are left out of the answer); skip
    /// (`AlreadyDone`) when `<base.request_id>:<branch>` already exists; else `BranchPricingPolicy::adjust` against
    /// the currency's anchor, write a row (`kind.row_source()`, `base_tier_set_id = base.id`,
    /// `request_id = <base.request_id>:<branch>`) and `TierSetPublisher::publish_in`. A failure rolls back only that
    /// branch and is reported as `Failed` with its error (`H:` §5).
    pub async fn push(
        &self,
        who: &Actor,
        base: &PricingTierSet,
        kind: PushKind,
    ) -> Vec<BranchResult> {
        let _ = (
            &self.pool,
            &self.publisher,
            &self.tier_sets,
            &self.policies,
            who,
            base,
            kind,
        );
        todo!("PV-d: BranchPush::push")
    }
}
