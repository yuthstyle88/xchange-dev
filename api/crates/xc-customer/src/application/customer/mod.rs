pub mod contact_reserve;
pub mod contact_verify;
pub mod phone_verification_service;
pub mod service;
mod system_phone;

pub use contact_reserve::ContactReserve;
pub use contact_verify::{ContactSummary, ContactVerify, ContactVerifyError};
pub use phone_verification_service::{
    PhoneConfirmResult, PhoneStartResult, PhoneVerificationError, PhoneVerificationService,
};
pub use service::CustomerService;
pub use system_phone::SystemReadPurpose;

impl CustomerService {
    pub async fn find_existing_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org_id: uuid::Uuid,
        lookup_hash: &[u8],
    ) -> pos_error::AppResult<Option<String>> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT customer_id FROM customer_identities \
             WHERE organization_id = $1 AND lookup_hash = $2 AND status = 'ACTIVE' \
             LIMIT 1",
        )
        .bind(org_id)
        .bind(lookup_hash)
        .fetch_optional(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(row)
    }

    pub async fn insert_customer_and_identity_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org_id: uuid::Uuid,
        customer_id: &str,
        display_name: &str,
        mode: xc_kernel::mode::Mode,
        created_at: chrono::DateTime<chrono::Utc>,
        identity: &crate::domain::customer::entities::customer_identity::CustomerIdentity,
    ) -> pos_error::AppResult<()> {
        crate::infrastructure::customer::customer_repository::PgCustomerRepository::insert_customer_and_identity_in_tx(
            tx,
            org_id,
            customer_id,
            display_name,
            mode,
            created_at,
            identity,
        )
        .await
    }
}
