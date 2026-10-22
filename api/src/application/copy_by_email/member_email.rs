//! `MemberEmails` (design `kiosk-shared-phone.md` §4.2 D21, W11): the member's verified, ACTIVE
//! `EMAIL` identity, the only address an e-mail copy is ever sent to.

use std::sync::Arc;

use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_customer::domain::sms_sender::BoxFuture;
use xc_kernel::pii::{PiiCipher, PiiKeys, Sealed};

/// A member's verified e-mail and the language its copy is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEmail {
    pub address: String,
    /// `customers.preferred_language`: `th`, `en` or `zh`.
    pub lang: String,
}

pub trait MemberEmails: Send + Sync {
    /// The member's `customer_identities` row of type `EMAIL`, `status = 'ACTIVE'` and
    /// `verified_at IS NOT NULL`, decrypted; `Ok(None)` when there is none (or several: the newest
    /// `verified_at` wins, `[proposed]`).
    fn verified_email<'a>(
        &'a self,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<VerifiedEmail>>>;
}

/// Reads `customer_identities` and decrypts with the PII keys (AAD `<customer_id>EMAIL`, as the
/// phone's is `<customer_id>PHONE`).
#[derive(Clone)]
pub struct PgMemberEmails {
    pool: PgPool,
    keys: Arc<PiiKeys>,
}

impl PgMemberEmails {
    pub fn new(pool: PgPool, keys: Arc<PiiKeys>) -> Self {
        Self { pool, keys }
    }
}

impl MemberEmails for PgMemberEmails {
    fn verified_email<'a>(
        &'a self,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<VerifiedEmail>>> {
        Box::pin(async move {
            let row: Option<(Vec<u8>, String, String)> = sqlx::query_as(
                "SELECT ci.encrypted_value, ci.key_id, c.preferred_language \
                 FROM customer_identities ci \
                 JOIN customers c ON c.id = ci.customer_id \
                 WHERE ci.customer_id = $1 \
                   AND ci.type = 'EMAIL' \
                   AND ci.status = 'ACTIVE' \
                   AND ci.verified_at IS NOT NULL \
                 ORDER BY ci.verified_at DESC \
                 LIMIT 1",
            )
            .bind(customer_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let Some((bytes, key_id, lang)) = row else {
                return Ok(None);
            };

            let plain = PiiCipher::new(self.keys.clone())
                .decrypt(
                    format!("{customer_id}EMAIL").as_bytes(),
                    &Sealed { key_id, bytes },
                )
                .map_err(|_| AppError::Internal("failed to decrypt verified email".to_owned()))?;
            let address = String::from_utf8(plain)
                .map_err(|_| AppError::Internal("verified email is not valid UTF-8".to_owned()))?;

            Ok(Some(VerifiedEmail { address, lang }))
        })
    }
}
