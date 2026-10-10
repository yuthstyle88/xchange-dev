//! PostgreSQL implementation of customer repository.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher};

use crate::domain::customer::entities::customer::Customer;
use crate::domain::customer::entities::customer_identity::CustomerIdentity;
use crate::domain::customer::repositories::customer_repository::CustomerRepository;
use crate::domain::customer::value_objects::customer_details::CustomerDetails;
use crate::pii_keys::get_pii_keys;

pub const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";

type CustomerDbRow = (
    String,
    String,
    String,
    String,
    DateTime<Utc>,
    Option<String>,
);

fn map_customer_row(row: CustomerDbRow) -> Customer {
    let (id, name, nationality, mode_str, created_at, last_document_ref) = row;
    let mode = match mode_str.as_str() {
        "DEMO" => Mode::Demo,
        _ => Mode::Live,
    };
    Customer {
        id,
        name,
        nationality,
        mode,
        created_at,
        last_document_ref,
    }
}

#[derive(Clone)]
pub struct PgCustomerRepository {
    pool: PgPool,
}

impl PgCustomerRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn organization_of_branch(&self, branch_id: &BranchId) -> AppResult<Uuid> {
        let org: Option<Uuid> =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
                .bind(branch_id.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
        org.ok_or_else(|| AppError::NotFound {
            entity: "branch",
            id: branch_id.to_string(),
        })
    }

    pub async fn find_existing_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org_id: Uuid,
        lookup_hash: &[u8],
    ) -> AppResult<Option<String>> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT customer_id FROM customer_identities \
             WHERE organization_id = $1 AND type = 'PASSPORT' AND lookup_hash = $2 AND status = 'ACTIVE' \
             LIMIT 1",
        )
        .bind(org_id)
        .bind(lookup_hash)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(row)
    }

    pub async fn increment_counter_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        key: &str,
    ) -> AppResult<Option<i64>> {
        let value: Option<i64> = sqlx::query_scalar(
            "INSERT INTO counters (key, value) VALUES ($1, 1) \
             ON CONFLICT (key) DO UPDATE SET value = counters.value + 1 WHERE counters.value < 999999999 \
             RETURNING value",
        )
        .bind(key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(value)
    }

    pub async fn insert_customer_and_identity_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org_id: Uuid,
        customer_id: &str,
        display_name: &str,
        mode: Mode,
        created_at: DateTime<Utc>,
        identity: &CustomerIdentity,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO customers ( \
                 id, organization_id, customer_no, display_name, \
                 preferred_language, status, mode, user_id, created_at, updated_at, last_document_ref \
             ) VALUES ($1, $2, $3, $4, 'th', 'ACTIVE', $5, NULL, $6, NULL, NULL)",
        )
        .bind(customer_id)
        .bind(org_id)
        .bind(customer_id)
        .bind(display_name)
        .bind(mode.as_str())
        .bind(created_at)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

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
        .map_err(AppError::Database)?;

        Ok(())
    }

    pub async fn confirm_customer_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &BranchId,
        customer_id: &str,
        details: &CustomerDetails,
    ) -> AppResult<()> {
        let keys = get_pii_keys()?;
        let normalized = xc_kernel::pii::normalize(
            IdentityType::Passport,
            &details.passport_no,
            Some(&details.nationality),
        )
        .map_err(|_| AppError::Domain(NOT_THIS_PASSPORT.to_owned()))?;
        let hash = LookupHasher::new(&keys).hash(IdentityType::Passport, &normalized);

        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM customers c \
                 JOIN customer_identities ci ON ci.customer_id = c.id \
                 JOIN branches b ON b.organization_id = c.organization_id \
                 WHERE b.id = $1 \
                   AND c.id = $2 \
                   AND ci.type = 'PASSPORT' \
                   AND ci.lookup_hash = $3 \
                   AND ci.status = 'ACTIVE' \
             )",
        )
        .bind(branch_id.as_str())
        .bind(customer_id)
        .bind(hash.as_slice())
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        if exists {
            Ok(())
        } else {
            Err(AppError::Domain(NOT_THIS_PASSPORT.to_owned()))
        }
    }

    pub async fn record_exchange_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &BranchId,
        customer_id: &str,
        details: &CustomerDetails,
        document_ref: &str,
    ) -> AppResult<()> {
        Self::confirm_customer_in_tx(tx, branch_id, customer_id, details).await?;

        sqlx::query(
            "UPDATE customers \
             SET display_name = $1, last_document_ref = $2, updated_at = now() \
             WHERE id = $3",
        )
        .bind(&details.name)
        .bind(document_ref)
        .bind(customer_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        let keys = get_pii_keys()?;
        let cipher = PiiCipher::new(keys);
        let payload = serde_json::json!({
            "number": details.passport_no,
            "nationality": details.nationality,
            "birthDate": details.birth_date.map(|d| d.to_string()),
            "expiry": details.passport_expiry.map(|d| d.to_string()),
            "nameOnDocument": details.name,
        });
        let aad = format!("{customer_id}PASSPORT");
        let sealed = cipher.encrypt(aad.as_bytes(), payload.to_string().as_bytes());

        sqlx::query(
            "UPDATE customer_identities \
             SET encrypted_value = $1, key_id = $2, updated_at = now() \
             WHERE customer_id = $3 AND type = 'PASSPORT' AND status = 'ACTIVE'",
        )
        .bind(&sealed.bytes)
        .bind(&sealed.key_id)
        .bind(customer_id)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }
}

impl CustomerRepository for PgCustomerRepository {
    async fn branch_exists(&self, branch_id: &BranchId) -> AppResult<bool> {
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM branches WHERE id = $1")
            .bind(branch_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(exists.is_some())
    }

    async fn find(&self, branch_id: &BranchId, id: &str) -> AppResult<Option<Customer>> {
        let row: Option<CustomerDbRow> = sqlx::query_as(
            "SELECT c.id, c.display_name, \
                    COALESCE((SELECT ci.nationality FROM customer_identities ci WHERE ci.customer_id = c.id AND ci.type = 'PASSPORT' AND ci.status = 'ACTIVE' LIMIT 1), '') AS nationality, \
                    c.mode, c.created_at, c.last_document_ref \
             FROM customers c \
             JOIN branches b ON b.organization_id = c.organization_id \
             WHERE b.id = $1 AND c.id = $2",
        )
        .bind(branch_id.as_str())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row.map(map_customer_row))
    }

    async fn list_live(&self, branch_id: &BranchId) -> AppResult<Vec<Customer>> {
        let rows: Vec<CustomerDbRow> = sqlx::query_as(
            "SELECT c.id, c.display_name, \
                    COALESCE((SELECT ci.nationality FROM customer_identities ci WHERE ci.customer_id = c.id AND ci.type = 'PASSPORT' AND ci.status = 'ACTIVE' LIMIT 1), '') AS nationality, \
                    c.mode, c.created_at, c.last_document_ref \
             FROM customers c \
             JOIN branches b ON b.organization_id = c.organization_id \
             WHERE b.id = $1 AND c.mode = 'LIVE' \
             ORDER BY c.created_at ASC, c.id ASC",
        )
        .bind(branch_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(rows.into_iter().map(map_customer_row).collect())
    }
}
