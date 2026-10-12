//! `LoyaltyFactsSource` (design §6): reads transaction and reversal facts for Loyalty event delivery.

use futures::future::BoxFuture;
use pos_error::AppResult;
use uuid::Uuid;

use crate::domain::facts::{CompletedFacts, ReversedFacts};

pub trait LoyaltyFactsSource: Send + Sync {
    /// Read facts of a completed exchange by transaction id.
    fn completed(&self, tx_id: Uuid) -> BoxFuture<'_, AppResult<CompletedFacts>>;

    /// Read facts of a reversal by reversal id.
    fn reversed(&self, rev_id: Uuid) -> BoxFuture<'_, AppResult<ReversedFacts>>;
}
