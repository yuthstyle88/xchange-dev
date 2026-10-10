//! `BookingMemberKeys` (design §6.2): today's `FeedCustomerKeys`. The phone of the booking's
//! member when the exchange came from a booking with `member_id` (`members.verified_at` is NOT NULL,
//! so a member's phone was checked by staff); otherwise no keys. `customer_sub` is always `None`
//! until #554 swaps in its own implementation.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::domain::customer_keys::{CustomerKeys, FeedCustomerKeys};

pub struct BookingMemberKeys;

impl FeedCustomerKeys for BookingMemberKeys {
    fn keys_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        transaction_id: Uuid,
    ) -> BoxFuture<'a, AppResult<CustomerKeys>> {
        Box::pin(async move {
            let phone: Option<String> = sqlx::query_scalar(
                "SELECT m.phone \
                 FROM transactions t \
                 JOIN bookings b ON b.transaction_ref = t.document_ref \
                 JOIN members m ON m.id = b.member_id \
                 WHERE t.id = $1::uuid AND m.verified_at IS NOT NULL \
                 LIMIT 1",
            )
            .bind(transaction_id.to_string())
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(CustomerKeys {
                customer_sub: None,
                phone,
            })
        })
    }
}
