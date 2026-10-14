//! `ContactVerify` (BK-10, booking-slots design §6 `members` row, §9 BK-10): staff verify the
//! customer's phone at the desk. The result is the customer's ACTIVE `PHONE` identity with
//! `verified_at` set, the company-wide member flag (PR 639 K1). Rulings on #1412
//! (issuecomment-6138038554): verifying the same phone twice keeps one identity and its first
//! `verified_at`; a phone ACTIVE for another customer of the organisation is `ValidationFailed`
//! on `phone` and writes nothing. The phone is sealed and hashed only through `xc-kernel::pii`
//! (K3); the audit row never holds it.
//!
//! Scaffold written with the up-front tests: the signatures only. BK-10's implementer writes the
//! bodies.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde::Serialize;
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

/// The customer's verified phone as the desk may see it: never the whole number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactSummary {
    pub customer_id: String,
    pub identity_id: Uuid,
    /// The phone with every digit but the last four hidden.
    pub phone_masked: String,
    pub verified_at: DateTime<Utc>,
}

/// Why a verification was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContactVerifyError {
    /// The phone is not a phone number, or it is ACTIVE for another customer of the organisation.
    /// Always about the `phone` field.
    #[error("validationFailed: {0}")]
    ValidationFailed(String),
    /// The customer is not one of the acting branch's organisation.
    #[error("outOfScope")]
    OutOfScope,
    #[error("internal: {0}")]
    Internal(String),
}

pub struct ContactVerify {
    branch_tx: BranchTx,
}

impl ContactVerify {
    pub fn new(branch_tx: BranchTx) -> Self {
        Self { branch_tx }
    }

    pub fn branch_tx(&self) -> &BranchTx {
        &self.branch_tx
    }

    /// Verifies `phone` as `customer_id`'s phone inside the caller's transaction, as `who` acting
    /// at `branch`.
    pub fn verify_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        who: &'a Actor,
        branch: &'a BranchId,
        customer_id: &'a str,
        phone: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<ContactSummary, ContactVerifyError>> {
        Box::pin(async move {
            let customer_org: Option<Uuid> =
                sqlx::query_scalar("SELECT organization_id FROM customers WHERE id = $1")
                    .bind(customer_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| ContactVerifyError::Internal(e.to_string()))?;

            let branch_org: Option<Uuid> =
                sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| ContactVerifyError::Internal(e.to_string()))?;

            let (Some(c_org), Some(b_org)) = (customer_org, branch_org) else {
                return Err(ContactVerifyError::OutOfScope);
            };
            if c_org != b_org {
                return Err(ContactVerifyError::OutOfScope);
            }
            let org_id = c_org;

            let normalized =
                xc_kernel::pii::normalize(xc_kernel::pii::IdentityType::Phone, phone, None)
                    .map_err(|e| {
                        ContactVerifyError::ValidationFailed(format!("invalid phone: {e}"))
                    })?;

            let keys =
                crate::get_pii_keys().map_err(|e| ContactVerifyError::Internal(e.to_string()))?;
            let lookup_hash = xc_kernel::pii::LookupHasher::new(&keys)
                .hash(xc_kernel::pii::IdentityType::Phone, &normalized);

            let existing: Option<(Uuid, String, Option<DateTime<Utc>>)> = sqlx::query_as(
                "SELECT id, customer_id, verified_at FROM customer_identities \
                 WHERE organization_id = $1 AND type = 'PHONE' AND lookup_hash = $2 AND status = 'ACTIVE' \
                 FOR UPDATE",
            )
            .bind(org_id)
            .bind(&lookup_hash[..])
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| ContactVerifyError::Internal(e.to_string()))?;

            let phone_masked = xc_kernel::pii::Masked::of(&normalized).to_string();

            if let Some((existing_id, existing_customer, existing_verified_at)) = existing {
                if existing_customer != customer_id {
                    return Err(ContactVerifyError::ValidationFailed(
                        "phone number already in use by another customer".to_string(),
                    ));
                }

                let verified_at = match existing_verified_at {
                    Some(v) => v,
                    None => {
                        sqlx::query(
                            "UPDATE customer_identities SET verified_at = $1, updated_at = $1 WHERE id = $2",
                        )
                        .bind(now)
                        .bind(existing_id)
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| ContactVerifyError::Internal(e.to_string()))?;

                        record_audit_event(tx, who, branch, customer_id, existing_id, now).await?;

                        now
                    }
                };

                return Ok(ContactSummary {
                    customer_id: customer_id.to_string(),
                    identity_id: existing_id,
                    phone_masked,
                    verified_at,
                });
            }

            sqlx::query(
                "UPDATE customer_identities \
                 SET status = 'REVOKED', updated_at = $3 \
                 WHERE organization_id = $1 AND customer_id = $2 AND type = 'PHONE' AND status = 'ACTIVE'",
            )
            .bind(org_id)
            .bind(customer_id)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|e| ContactVerifyError::Internal(e.to_string()))?;

            let cipher = xc_kernel::pii::PiiCipher::new(keys);
            let aad = format!("{customer_id}PHONE");
            let sealed = cipher.encrypt(aad.as_bytes(), normalized.as_bytes());
            let new_id = Uuid::new_v4();

            sqlx::query(
                "INSERT INTO customer_identities ( \
                     id, organization_id, customer_id, type, encrypted_value, key_id, \
                     lookup_hash, nationality, verified_at, status, created_at, updated_at \
                 ) VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, NULL, $7, 'ACTIVE', $7, NULL)",
            )
            .bind(new_id)
            .bind(org_id)
            .bind(customer_id)
            .bind(&sealed.bytes)
            .bind(&sealed.key_id)
            .bind(&lookup_hash[..])
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(|e| ContactVerifyError::Internal(e.to_string()))?;

            record_audit_event(tx, who, branch, customer_id, new_id, now).await?;

            Ok(ContactSummary {
                customer_id: customer_id.to_string(),
                identity_id: new_id,
                phone_masked,
                verified_at: now,
            })
        })
    }
}

async fn record_audit_event(
    conn: &mut PgConnection,
    who: &Actor,
    branch: &BranchId,
    customer_id: &str,
    identity_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), ContactVerifyError> {
    let role_str = who
        .roles
        .first()
        .map(|r| r.role.as_str())
        .unwrap_or("TELLER");

    sqlx::query(
        "INSERT INTO audit_events ( \
             actor_id, actor_role, scope, branch_id, action, \
             object_ref, before_redacted, after_redacted, at \
         ) VALUES ($1, $2, 'BRANCH', $3, 'customerContactVerify', $4, NULL, $5, $6)",
    )
    .bind(&who.user_id)
    .bind(role_str)
    .bind(branch.as_str())
    .bind(customer_id)
    .bind(Some(serde_json::json!({ "identity_id": identity_id })))
    .bind(now)
    .execute(conn)
    .await
    .map_err(|e| ContactVerifyError::Internal(e.to_string()))?;

    Ok(())
}
