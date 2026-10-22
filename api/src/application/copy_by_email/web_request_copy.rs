//! `WebRequestCopy` (design `kiosk-shared-phone.md` §4.2 D21): the web request subject of the
//! e-mail copy. One read of `kiosk_requests` by member (`customer_id`, slice a's migration) and
//! request token hash (`cancel_token_hash`, the lowercase hex SHA-256 of the token, D2).

use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
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

    async fn load(
        &self,
        member: Option<&str>,
        id: &str,
        qr_hash: &[u8; 32],
    ) -> AppResult<Option<CopyDetails>> {
        let token_hash: String = qr_hash.iter().map(|b| format!("{b:02x}")).collect();
        let row = sqlx::query(
            "SELECT r.id, br.name, r.status, t.status, \
                    btrim(r.currency), r.foreign_amount::text, r.rate::text, r.created_at \
             FROM kiosk_requests r \
             JOIN branches br ON br.id = r.branch_id \
             LEFT JOIN queue_tickets t ON t.branch_id = r.branch_id AND t.kiosk_request_id = r.id \
             WHERE r.id = $1 \
               AND r.cancel_token_hash = $2 \
               AND ($3::text IS NULL OR r.customer_id = $3)",
        )
        .bind(id)
        .bind(&token_hash)
        .bind(member)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let request_id: String = row.get(0);
        let branch_name: String = row.get(1);
        let request_status: String = row.get(2);
        let ticket_status: Option<String> = row.get(3);
        let currency: String = row.get(4);
        let amount: String = row.get(5);
        let rate: String = row.get(6);
        let created_at: DateTime<Utc> = row.get(7);

        let expires_at = (created_at + Duration::minutes(60)).to_rfc3339();
        let status = w5_status(&request_status, ticket_status.as_deref());

        Ok(Some(CopyDetails {
            kind: CopySubjectKind::WebRequest,
            id: request_id,
            branch_name,
            rows: vec![
                ("currency".to_owned(), currency),
                ("amount".to_owned(), amount),
                ("rate".to_owned(), rate),
                ("expiresAt".to_owned(), expires_at),
            ],
            status,
        }))
    }
}

fn w5_status(request_status: &str, ticket_status: Option<&str>) -> String {
    match request_status {
        "ACCEPTED" | "ID_CHECKED" => "SERVING".to_owned(),
        "COMPLETED" | "CANCELLED" => request_status.to_owned(),
        _ => ticket_status.unwrap_or(request_status).to_owned(),
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
        Box::pin(async move { self.load(Some(member), id, qr_hash).await })
    }

    /// The same row by `id` and token hash only.
    fn load_by_link<'a>(
        &'a self,
        id: &'a str,
        qr_hash: &'a [u8; 32],
    ) -> BoxFuture<'a, AppResult<Option<CopyDetails>>> {
        Box::pin(async move { self.load(None, id, qr_hash).await })
    }
}
