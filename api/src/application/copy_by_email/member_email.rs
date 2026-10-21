//! `MemberEmails` (design `kiosk-shared-phone.md` §4.2 D21, W11): the member's verified, ACTIVE
//! `EMAIL` identity, the only address an e-mail copy is ever sent to.

use std::sync::Arc;

use pos_error::AppResult;
use sqlx::PgPool;
use xc_customer::domain::sms_sender::BoxFuture;
use xc_kernel::pii::PiiKeys;

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
        let _ = (&self.pool, &self.keys, customer_id);
        Box::pin(async move { todo!("KWQ-D4: the verified ACTIVE EMAIL identity, decrypted") })
    }
}
