//! CU-L1 (#1376, membership-points design D-15): the system read of a customer's verified phone.
//!
//! SCAFFOLD written by the ticket author (claude/29df2ec5) so the up-front tests compile on `main`.
//! The signatures are the ticket's `[required]` ones; the bodies are deliberately wrong placeholders
//! (an error, and an unaudited `None`) that the implementer replaces.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;

use super::service::CustomerService;

/// Why a system caller reads a verified phone. One variant only, so no caller names an ad-hoc
/// purpose (design D-15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemReadPurpose {
    Loyalty,
}

impl SystemReadPurpose {
    /// The purpose as recorded in the audit row (`actor_id = "system:<purpose>"`, reason).
    pub fn as_str(&self) -> &'static str {
        match self {
            SystemReadPurpose::Loyalty => "loyalty",
        }
    }
}

impl CustomerService {
    /// True when the customer has an ACTIVE PHONE identity with `verified_at` set in the
    /// organisation. Never decrypts.
    pub async fn has_verified_phone_in(
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        customer_id: &str,
    ) -> AppResult<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM customer_identities \
                 WHERE organization_id = $1 \
                   AND customer_id = $2 \
                   AND type = 'PHONE' \
                   AND status = 'ACTIVE' \
                   AND verified_at IS NOT NULL \
             )",
        )
        .bind(organization_id)
        .bind(customer_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(exists)
    }

    /// The E.164 value of the customer's ACTIVE verified PHONE, `None` otherwise; every call writes
    /// one `audit_events` row in the same transaction as the read.
    pub async fn verified_phone_for_system(
        &self,
        organization_id: Uuid,
        customer_id: &str,
        purpose: SystemReadPurpose,
        _now: DateTime<Utc>,
    ) -> AppResult<Option<String>> {
        let mut tx = self
            .branch_tx()
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;

        let row: Option<(Uuid, Vec<u8>, String)> = sqlx::query_as(
            "SELECT id, encrypted_value, key_id FROM customer_identities \
             WHERE organization_id = $1 \
               AND customer_id = $2 \
               AND type = 'PHONE' \
               AND status = 'ACTIVE' \
               AND verified_at IS NOT NULL \
             ORDER BY id DESC \
             LIMIT 1",
        )
        .bind(organization_id)
        .bind(customer_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AppError::Database)?;

        let (phone_opt, object_ref) = match row {
            Some((identity_id, encrypted_value, key_id)) => {
                let keys = crate::pii_keys::get_pii_keys()?;
                let cipher = xc_kernel::pii::PiiCipher::new(keys);
                let aad = format!("{customer_id}PHONE");
                let sealed = xc_kernel::pii::Sealed {
                    key_id,
                    bytes: encrypted_value,
                };
                let decrypted = cipher
                    .decrypt(aad.as_bytes(), &sealed)
                    .map_err(|e| AppError::Internal(format!("decrypt failed: {e}")))?;

                let phone_raw = std::str::from_utf8(&decrypted).map_err(|e| {
                    AppError::Internal(format!("invalid UTF-8 in decrypted phone: {e}"))
                })?;

                let normalized =
                    xc_kernel::pii::normalize(xc_kernel::pii::IdentityType::Phone, phone_raw, None)
                        .map_err(|e| AppError::Internal(format!("phone normalize failed: {e}")))?;

                (Some(normalized), identity_id.to_string())
            }
            None => (None, customer_id.to_string()),
        };

        let event = AuditEvent {
            actor_id: format!("system:{}", purpose.as_str()),
            actor_role: "SYSTEM".to_string(),
            scope: "SYSTEM".to_string(),
            action: "identity.system_read".to_string(),
            object_ref,
            branch_id: None,
            before: None,
            after: Some(serde_json::json!({
                "reason": purpose.as_str(),
            })),
        };

        let audit_log = PgAuditLog::new(self.branch_tx().pool().clone());
        audit_log.append(&mut tx, &event).await?;

        tx.commit().await.map_err(AppError::Database)?;

        Ok(phone_opt)
    }
}
