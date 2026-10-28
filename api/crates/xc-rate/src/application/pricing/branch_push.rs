//! MAIN is the base; branches follow it in basis points (pricing-tiers.md §15.4; `H:` §5, §7).
//!
//! SKELETON (XC-V9-PRICING): `BranchPush::push` is PV-d1's; the wire strings are written here.
//! Types and signatures stay.

use std::sync::Arc;

use chrono::Utc;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_auth::domain::access::Actor;

use crate::application::pricing::tier_publish::TierSetPublisher;
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::branch_policy::{BranchPolicyStore, BranchPricingPolicy};
use crate::domain::pricing::market_quote::MarketQuote;
use crate::domain::pricing::pricing_suggestion::SuggestionConfig;
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
        match self {
            PushKind::Manual => TierSetSource::MainPush,
            PushKind::Follow => TierSetSource::Follow,
        }
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
        match self {
            BranchPushState::Published => "PUBLISHED",
            BranchPushState::SkippedOwn => "SKIPPED_OWN",
            BranchPushState::Failed => "FAILED",
            BranchPushState::AlreadyDone => "ALREADY_DONE",
        }
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
        let branches: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM branches WHERE id <> 'MAIN' AND status = 'ACTIVE' ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let anchor_quote = base.anchor.as_ref().map(|a| MarketQuote {
            source: a.source,
            buy: Some(a.buy),
            sell: Some(a.sell),
            unit_size: Decimal::ONE,
            fetched_at: a.fetched_at,
            stale: false,
        });
        let config = SuggestionConfig::v9_default();

        let mut results = Vec::with_capacity(branches.len());
        for branch in branches {
            let mut tx = match self.pool.begin().await {
                Ok(tx) => tx,
                Err(_) => {
                    results.push(BranchResult {
                        branch,
                        state: BranchPushState::Failed,
                        tier_set_id: None,
                        error: Some("serverError".to_owned()),
                    });
                    continue;
                }
            };

            match self
                .push_branch(
                    &mut tx,
                    &branch,
                    who,
                    base,
                    kind,
                    anchor_quote.as_ref(),
                    &config,
                )
                .await
            {
                Ok(Some(res)) => {
                    if res.state == BranchPushState::Published && tx.commit().await.is_err() {
                        results.push(BranchResult {
                            branch,
                            state: BranchPushState::Failed,
                            tier_set_id: None,
                            error: Some("serverError".to_owned()),
                        });
                        continue;
                    }
                    results.push(res);
                }
                Ok(None) => {}
                Err(code) => {
                    let _ = tx.rollback().await;
                    results.push(BranchResult {
                        branch,
                        state: BranchPushState::Failed,
                        tier_set_id: None,
                        error: Some(code.to_owned()),
                    });
                }
            }
        }
        results
    }

    #[allow(clippy::too_many_arguments)]
    async fn push_branch(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &str,
        who: &Actor,
        base: &PricingTierSet,
        kind: PushKind,
        anchor: Option<&MarketQuote>,
        config: &SuggestionConfig,
    ) -> Result<Option<BranchResult>, &'static str> {
        let policy = self
            .policies
            .for_branch_in(tx, branch)
            .await
            .map_err(map_err_code)?;
        let resolved = policy.resolve(base.currency.as_str());

        if kind == PushKind::Follow && !resolved.follow {
            return Ok(None);
        }
        if resolved.own {
            return Ok(Some(BranchResult {
                branch: branch.to_owned(),
                state: BranchPushState::SkippedOwn,
                tier_set_id: None,
                error: None,
            }));
        }

        let request_id = format!("{}:{branch}", base.request_id);
        if let Some(existing) = self
            .tier_sets
            .by_request_in(tx, &request_id)
            .await
            .map_err(map_err_code)?
        {
            return Ok(Some(BranchResult {
                branch: branch.to_owned(),
                state: BranchPushState::AlreadyDone,
                tier_set_id: Some(existing.id),
                error: None,
            }));
        }

        let unit_size: Decimal =
            sqlx::query_scalar("SELECT unit_size FROM currencies WHERE code = $1")
                .bind(base.currency.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| map_err_code(RateEngineError::Database(e.to_string())))?
                .unwrap_or(Decimal::ONE);

        let tiers = BranchPricingPolicy::adjust(&base.tiers, &resolved, unit_size, anchor, config)
            .map_err(|_| "validationFailed")?;

        let set = PricingTierSet {
            id: Uuid::new_v4(),
            request_id: request_id.clone(),
            branch: branch.to_owned(),
            currency: base.currency,
            source: kind.row_source(),
            base_tier_set_id: Some(base.id),
            tiers,
            anchor: base.anchor.clone(),
            flags: base.flags.clone(),
            draft_id: None,
            approved_by: who.user_id.clone(),
            approved_at: Utc::now(),
        };

        let inserted = self
            .tier_sets
            .insert_in(tx, &set)
            .await
            .map_err(map_err_code)?;
        if !inserted {
            return Ok(Some(BranchResult {
                branch: branch.to_owned(),
                state: BranchPushState::AlreadyDone,
                tier_set_id: None,
                error: None,
            }));
        }

        self.publisher
            .publish_in(tx, who, &set, &request_id)
            .await
            .map_err(map_err_code)?;

        Ok(Some(BranchResult {
            branch: branch.to_owned(),
            state: BranchPushState::Published,
            tier_set_id: Some(set.id),
            error: None,
        }))
    }
}

fn map_err_code(err: RateEngineError) -> &'static str {
    match err {
        RateEngineError::Database(_) | RateEngineError::Internal(_) => "serverError",
        other => other.code(),
    }
}
