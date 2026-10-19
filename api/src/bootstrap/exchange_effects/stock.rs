//! B05 stock effect (design `docs/design/blueprint/stock-reservation.md` D-5, §5): a completion consumes its quote's
//! live reservations in phase `Consume`, before `post_in`.
//!
//! Skeleton by the B05 sub-controller (claude/6530d490) on `feat/b05-stock`. `effect` answers `None` (not
//! registered) until SR-2 turns it on, so no completion reaches the `todo!()` before then.

use std::sync::Arc;

use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_cash::domain::reservation::port::StockReservations;
use xc_cash::infrastructure::stock_reservations::PgStockReservations;

use crate::application::workflows::exchange::effects::{
    BoxFuture, EffectPhase, ExchangeCompletedFacts, ExchangeEffect, PostedReversal,
};
use crate::bootstrap::AppState;

pub struct StockConsumeEffect {
    reservations: Arc<dyn StockReservations>,
}

impl StockConsumeEffect {
    pub fn new(reservations: Arc<dyn StockReservations>) -> Self {
        Self { reservations }
    }
}

impl ExchangeEffect for StockConsumeEffect {
    fn name(&self) -> &'static str {
        "stock"
    }

    fn phase(&self) -> EffectPhase {
        EffectPhase::Consume
    }

    /// D-5: `consume_for_quote_in(quote_id, transaction_id, completed_at)`.
    fn on_completed_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        e: &'a ExchangeCompletedFacts,
    ) -> BoxFuture<'a, AppResult<()>> {
        let _ = (&self.reservations, tx, e);
        todo!("SR-2")
    }

    /// A reversal returns stock through its own posting; a consumed reservation is history only (§6 item 4).
    fn on_reversed_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _r: &'a PostedReversal,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// `None` in the skeleton (not registered); SR-2 answers `Some(Arc::new(stock))`.
pub fn effect(_state: &AppState) -> Option<Arc<dyn ExchangeEffect>> {
    let _stock = StockConsumeEffect::new(Arc::new(PgStockReservations::new()));
    None
}
