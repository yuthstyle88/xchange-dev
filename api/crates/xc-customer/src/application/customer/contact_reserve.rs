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

pub struct ContactReserve;

impl ContactReserve {
    pub fn new(_branch_tx: BranchTx) -> Self {
        Self
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

            let reserved = super::CustomerService::reserve_customer_number_in_tx(
                tx,
                org_id,
                &lookup_hash,
                xc_kernel::mode::Mode::Live,
                now,
            )
            .await?;

            let (customer_id, created_at) = match reserved {
                super::service::ReservedCustomerNumber::Existing(customer_id) => {
                    return Ok(CustomerReservation {
                        customer_id,
                        existing: true,
                    });
                }
                super::service::ReservedCustomerNumber::New {
                    customer_id,
                    created_at,
                } => (customer_id, created_at),
            };

            let cipher = xc_kernel::pii::PiiCipher::new(keys);
            let aad = format!("{customer_id}PHONE");
            let sealed = cipher.encrypt(aad.as_bytes(), normalized.as_bytes());

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
