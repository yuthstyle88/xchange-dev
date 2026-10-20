//! XS-0 #1375: the exchange-effect seam (design `docs/design/blueprint/membership-points.md` §6.2): what else
//! happens, inside the caller's transaction, when an exchange completes (TX-5) or a reversal posts (RV-3).
//!
//! The names and signatures are the ones TX-5 merged as a stand-in in `complete.rs`, which now re-exports them
//! from here, so the trait has exactly one definition (#1375 §4 item 3). The effects themselves are registered by
//! file under `bootstrap/exchange_effects/` (one file per destination, file-name order).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;

/// §6.2's `BoxFuture<'a, AppResult<()>>`, spelled without the `futures` crate.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// §09's effect steps: `Consume` = "consume reservation" (B05 D-5, before the posting; may refuse), `Redeem` =
/// "redeem benefit" (may refuse), `Outbox` = "write outbox".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectPhase {
    Consume,
    Redeem,
    Outbox,
}

/// What a completion tells its effects (§6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeCompletedFacts {
    pub organization_id: Uuid,
    pub branch_id: BranchId,
    pub transaction_id: Uuid,
    pub quote_id: Uuid,
    pub customer_id: Option<String>,
    pub mode: Mode,
    pub completed_at: DateTime<Utc>,
    pub business_date: NaiveDate,
    pub reversible_until: DateTime<Utc>,
}

/// What a posted reversal tells its effects (§6.2; RV-3's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedReversal {
    pub organization_id: Uuid,
    pub reversal_id: Uuid,
    pub transaction_id: Uuid,
    pub customer_id: Option<String>,
    pub mode: Mode,
    pub posted_at: DateTime<Utc>,
    pub amount_thb: Decimal,
}

/// One registered effect of a completion or a reversal. An `Err` rolls back the caller's transaction.
pub trait ExchangeEffect: Send + Sync {
    fn name(&self) -> &'static str;
    fn phase(&self) -> EffectPhase;
    fn on_completed_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        e: &'a ExchangeCompletedFacts,
    ) -> BoxFuture<'a, AppResult<()>>;
    fn on_reversed_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        r: &'a PostedReversal,
    ) -> BoxFuture<'a, AppResult<()>>;
}

/// The registered effects, in registration order.
pub struct ExchangeEffects {
    effects: Vec<Arc<dyn ExchangeEffect>>,
}

impl ExchangeEffects {
    /// The list the generated registration passes.
    pub fn new(effects: Vec<Arc<dyn ExchangeEffect>>) -> Self {
        Self { effects }
    }

    /// Runs every effect of `phase`, in order, on `tx`; the first `Err` is the answer, unchanged, and the
    /// effects after it do not run (the caller rolls back).
    pub async fn run_completed_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        phase: EffectPhase,
        e: &ExchangeCompletedFacts,
    ) -> AppResult<()> {
        for effect in self.effects.iter().filter(|f| f.phase() == phase) {
            effect.on_completed_in(tx, e).await?;
        }
        Ok(())
    }

    /// RV-3's call; the same rule as [`Self::run_completed_in`].
    pub async fn run_reversed_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        phase: EffectPhase,
        r: &PostedReversal,
    ) -> AppResult<()> {
        for effect in self.effects.iter().filter(|f| f.phase() == phase) {
            effect.on_reversed_in(tx, r).await?;
        }
        Ok(())
    }
}
