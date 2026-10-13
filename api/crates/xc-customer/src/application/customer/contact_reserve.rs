//! `ContactReserve` (design §4.4, §5): reserves or finds customer by phone,
//! installing an unverified PHONE identity.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::branch_tx::BranchTx;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomerReservation {
    pub customer_id: String,
    pub existing: bool,
}

pub struct ContactReserve {
    #[allow(dead_code)]
    branch_tx: BranchTx,
}

impl ContactReserve {
    pub fn new(branch_tx: BranchTx) -> Self {
        Self { branch_tx }
    }

    pub fn reserve_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        org_id: Uuid,
        display_name: &'a str,
        phone: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<CustomerReservation, pos_error::AppError>> {
        Box::pin(async move {
            let keys = crate::pii_keys::get_pii_keys()?;
            let normalized =
                xc_kernel::pii::normalize(xc_kernel::pii::IdentityType::Phone, phone, None)
                    .map_err(|e| pos_error::AppError::Domain(format!("invalid phone: {e}")))?;

            let lookup_hash = xc_kernel::pii::LookupHasher::new(&keys)
                .hash(xc_kernel::pii::IdentityType::Phone, &normalized);

            let lock_key = i64::from_be_bytes(lookup_hash[0..8].try_into().unwrap());
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(lock_key)
                .execute(&mut **tx)
                .await
                .map_err(pos_error::AppError::Database)?;

            if let Some(customer_id) =
                super::CustomerService::find_existing_in_tx(tx, org_id, &lookup_hash).await?
            {
                return Ok(CustomerReservation {
                    customer_id,
                    existing: true,
                });
            }

            let counter_key = "customer-LIVE";
            let next_val = crate::infrastructure::customer::customer_repository::PgCustomerRepository::increment_counter_in_tx(tx, counter_key)
                .await?
                .ok_or_else(|| pos_error::AppError::Domain("ตัวนับรหัสไม่ถูกต้อง".to_owned()))?;
            let customer_id = format!("CX-{next_val:08}");

            let created_at = chrono::TimeZone::timestamp_millis_opt(&Utc, now.timestamp_millis())
                .single()
                .unwrap_or(now);

            let cipher = xc_kernel::pii::PiiCipher::new(keys);
            let aad = format!("{customer_id}PHONE");
            let sealed = cipher.encrypt(aad.as_bytes(), phone.as_bytes());

            let new_identity =
                crate::domain::customer::entities::customer_identity::CustomerIdentity {
                    id: Uuid::new_v4(),
                    organization_id: org_id,
                    customer_id: customer_id.clone(),
                    r#type: xc_kernel::pii::IdentityType::Phone,
                    encrypted_value: sealed.bytes,
                    key_id: sealed.key_id,
                    lookup_hash: lookup_hash.to_vec(),
                    nationality: None,
                    verified_at: None,
                    status: "ACTIVE".to_string(),
                    created_at,
                    updated_at: None,
                };

            super::CustomerService::insert_customer_and_identity_in_tx(
                tx,
                org_id,
                &customer_id,
                display_name,
                xc_kernel::mode::Mode::Live,
                created_at,
                &new_identity,
            )
            .await?;

            Ok(CustomerReservation {
                customer_id,
                existing: false,
            })
        })
    }
}
