//! `BookingMemberKeys` (design §6.2): today's `FeedCustomerKeys`. The phone of the booking's
//! member when the exchange came from a Blueprint booking whose customer has a verified ACTIVE
//! `PHONE` identity (BK-10, ruling (b)); otherwise no keys. `customer_sub` is always `None`
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
            let row: Option<(String, Vec<u8>, String)> = sqlx::query_as(
                "SELECT ci.customer_id, ci.encrypted_value, ci.key_id \
                 FROM bookings b \
                 JOIN customer_identities ci ON ci.customer_id = b.customer_id AND ci.organization_id = b.organization_id \
                 WHERE b.transaction_id = $1 \
                   AND ci.type = 'PHONE' \
                   AND ci.status = 'ACTIVE' \
                   AND ci.verified_at IS NOT NULL \
                 LIMIT 1",
            )
            .bind(transaction_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            let Some((customer_id, encrypted_value, key_id)) = row else {
                return Ok(CustomerKeys {
                    customer_sub: None,
                    phone: None,
                });
            };

            let keys = xc_customer::get_pii_keys()
                .map_err(|e| pos_error::AppError::Internal(e.to_string()))?;
            let cipher = xc_kernel::pii::PiiCipher::new(keys);
            let aad = format!("{customer_id}PHONE");
            let phone_bytes = cipher
                .decrypt(
                    aad.as_bytes(),
                    &xc_kernel::pii::Sealed {
                        key_id,
                        bytes: encrypted_value,
                    },
                )
                .map_err(|e| {
                    pos_error::AppError::Internal(format!("failed to decrypt phone: {e}"))
                })?;
            let phone = String::from_utf8(phone_bytes).map_err(|e| {
                pos_error::AppError::Internal(format!("invalid utf8 in phone: {e}"))
            })?;

            Ok(CustomerKeys {
                customer_sub: None,
                phone: Some(phone),
            })
        })
    }
}
