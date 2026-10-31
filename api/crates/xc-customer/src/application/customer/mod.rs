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

    pub async fn insert_identity_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        identity: &crate::domain::customer::entities::customer_identity::CustomerIdentity,
    ) -> pos_error::AppResult<()> {
        sqlx::query(
            "INSERT INTO customer_identities ( \
                 id, organization_id, customer_id, type, encrypted_value, \
                 key_id, lookup_hash, nationality, verified_at, status, created_at, updated_at \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(identity.id)
        .bind(identity.organization_id)
        .bind(&identity.customer_id)
        .bind(identity.r#type.as_str())
        .bind(&identity.encrypted_value)
        .bind(&identity.key_id)
        .bind(&identity.lookup_hash)
        .bind(&identity.nationality)
        .bind(identity.verified_at)
        .bind(&identity.status)
        .bind(identity.created_at)
        .bind(identity.updated_at)
        .execute(&mut **tx)
        .await
        .map_err(pos_error::AppError::Database)?;

        Ok(())
    }
}
