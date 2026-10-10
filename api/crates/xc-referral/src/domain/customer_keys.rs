//! `FeedCustomerKeys` (design §6.2, D-6, condition 3): who the exchange's customer is, for Broker.
//! A phone leaves v3 only from a **verified** source. A row with neither key is still written.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomerKeys {
    pub customer_sub: Option<String>,
    /// From a verified source only, as stored.
    pub phone: Option<String>,
}

pub trait FeedCustomerKeys: Send + Sync {
    /// The keys of `transaction_id`, read in the caller's transaction.
    fn keys_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        transaction_id: Uuid,
    ) -> BoxFuture<'a, AppResult<CustomerKeys>>;
}
