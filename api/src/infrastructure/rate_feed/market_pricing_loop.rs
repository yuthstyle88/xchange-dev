//! FOLLOW: automatic market pricing at MAIN (pricing-tiers.md §15.5; `H:` §6). After every `MarketRefresh::tick`
//! (5 min, only when `MARKET_FEED_ENABLED=true`), plan each FOLLOW currency under `RATE_FEED_LOCK_KEY` and publish
//! the unblocked plans. It does not depend on the BoT tick (gap 5).
//!
//! SKELETON (XC-V9-PRICING): `run`, `plan_once` and `plan_currency` are PV-f's; the spawn in `bootstrap/mod.rs` is
//! PV-f's too. Types and signatures stay.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::watch;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::market::market_refresh::MarketRefresh;
use xc_rate::application::pricing::branch_push::BranchPush;
use xc_rate::application::pricing::tier_publish::TierSetPublisher;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::pricing::follow_plan::FollowResult;

pub struct MarketPricingLoop {
    pool: PgPool,
    refresh: Arc<MarketRefresh>,
    publisher: Arc<TierSetPublisher>,
    push: Arc<BranchPush>,
}

impl MarketPricingLoop {
    pub fn new(
        pool: PgPool,
        refresh: Arc<MarketRefresh>,
        publisher: Arc<TierSetPublisher>,
        push: Arc<BranchPush>,
    ) -> Self {
        Self {
            pool,
            refresh,
            publisher,
            push,
        }
    }

    /// `refresh.tick(now)` then `plan_once(now)`, every `interval`, until `stop` turns true.
    pub async fn run(self: Arc<Self>, interval: Duration, stop: watch::Receiver<bool>) {
        let _ = (&self.refresh, interval, stop);
        todo!("PV-f: MarketPricingLoop::run")
    }

    /// Plans every currency whose `pricing_modes.mode` is `FOLLOW`, under `RATE_FEED_LOCK_KEY`, and returns each
    /// result in currency order. See `plan_currency`.
    pub async fn plan_once(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<FollowResult>, RateEngineError> {
        let _ = now;
        todo!("PV-f: MarketPricingLoop::plan_once")
    }

    /// One FOLLOW currency at MAIN: build `FollowInput`, `FollowPlan::plan`; blocked ⇒ record `BLOCKED` with the
    /// reasons, publish nothing; a candidate ⇒ write a `FOLLOW` tier set with
    /// `request_id = follow:MAIN:<code>:<anchor batch id>` (already there ⇒ nothing new, the result stays as it was),
    /// `TierSetPublisher::publish_in` at MAIN, record `PUBLISHED`, then `BranchPush::push(.., PushKind::Follow)`.
    /// A failed publish records `CANDIDATE`.
    pub async fn plan_currency(
        &self,
        currency: CurrencyCode,
        now: DateTime<Utc>,
    ) -> Result<FollowResult, RateEngineError> {
        let _ = (&self.pool, &self.publisher, &self.push, currency, now);
        todo!("PV-f: MarketPricingLoop::plan_currency")
    }
}
