//! `WebRequestCopy` (design `kiosk-shared-phone.md` §4.2 D21): the web request subject of the
//! e-mail copy. One read of `kiosk_requests` by member (`customer_id`, slice a's migration) and
//! request token hash (`cancel_token_hash`, the lowercase hex SHA-256 of the token, D2).

use pos_error::AppResult;
use sqlx::PgPool;
use xc_customer::domain::sms_sender::BoxFuture;

use super::subject::{CopyDetails, CopySubject, CopySubjectKind};

#[derive(Clone)]
pub struct WebRequestCopy {
    pool: PgPool,
}

impl WebRequestCopy {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl CopySubject for WebRequestCopy {
    fn kind(&self) -> CopySubjectKind {
        CopySubjectKind::WebRequest
    }

    /// `kiosk_requests` row `id` with `customer_id = member` and `cancel_token_hash =
    /// hex(qr_hash)`. Rows (`[proposed]`, in order): `currency`, `amount` (the foreign amount),
    /// `rate`, `expiresAt` (`created_at` + 60 min, D4, RFC 3339); `status` = W5's status (from the
    /// request and its ticket); `branch_name` = `branches.name`. A purged request has no row ⇒
    /// `None`.
    fn load_for<'a>(
        &'a self,
        member: &'a str,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        let _ = (&self.pool, member, id, qr_hash);
        Box::pin(async move { todo!("KWQ-D6: the member's web request by token hash") })
    }

    /// The same row by `id` and token hash only.
    fn load_by_link<'a>(
        &'a self,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        let _ = (&self.pool, id, qr_hash);
        Box::pin(async move { todo!("KWQ-D6: the web request by token hash") })
    }
}
