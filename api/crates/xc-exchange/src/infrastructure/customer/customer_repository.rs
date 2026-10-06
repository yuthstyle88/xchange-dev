//! PostgreSQL implementation of customer repository.

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;

use crate::domain::customer::entities::customer::Customer;
use crate::domain::customer::repositories::customer_repository::CustomerRepository;
use crate::domain::customer::value_objects::customer_details::CustomerDetails;
use crate::domain::customer::value_objects::mode::Mode;

pub const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";

type CustomerDbRow = (
    String,
    String,
    String,
    String,
    String,
    Option<NaiveDate>,
    Option<NaiveDate>,
    String,
    DateTime<Utc>,
    Option<String>,
);

fn map_customer_row(row: CustomerDbRow) -> Customer {
    let (
        id,
        branch_id,
        name,
        passport_no,
        nationality,
        birth_date,
        passport_expiry,
        mode_str,
        created_at,
        last_document_ref,
    ) = row;
    let mode = match mode_str.as_str() {
        "DEMO" => Mode::Demo,
        _ => Mode::Live,
    };
    Customer {
        id,
        branch_id: BranchId::new(branch_id),
        name,
        passport_no,
        nationality,
        birth_date,
        passport_expiry,
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

    pub async fn find_existing_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &BranchId,
        passport_no: &str,
        nationality: &str,
        mode: Mode,
    ) -> AppResult<Option<String>> {
        let row: Option<String> = sqlx::query_scalar(
            "SELECT id FROM customers \
             WHERE branch_id = $1 AND passport_no = $2 AND nationality = $3 AND mode = $4",
        )
        .bind(branch_id.as_str())
        .bind(passport_no)
        .bind(nationality)
        .bind(mode.as_str())
        .fetch_optional(&mut **tx)
        .await?;

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
        .await?;

        Ok(value)
    }

    pub async fn insert_customer_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        customer: &Customer,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO customers (id, branch_id, name, passport_no, nationality, birth_date, passport_expiry, mode, created_at, last_document_ref) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL)",
        )
        .bind(&customer.id)
        .bind(customer.branch_id.as_str())
        .bind(&customer.name)
        .bind(&customer.passport_no)
        .bind(&customer.nationality)
        .bind(customer.birth_date)
        .bind(customer.passport_expiry)
        .bind(customer.mode.as_str())
        .bind(customer.created_at)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    pub async fn confirm_customer_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &BranchId,
        customer_id: &str,
        details: &CustomerDetails,
    ) -> AppResult<()> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT passport_no, nationality, mode FROM customers WHERE branch_id = $1 AND id = $2",
        )
        .bind(branch_id.as_str())
        .bind(customer_id)
        .fetch_optional(&mut **tx)
        .await?;

        match row {
            Some((passport_no, nationality, mode))
                if passport_no == details.passport_no
                    && nationality == details.nationality
                    && mode == details.mode.as_str() =>
            {
                Ok(())
            }
            _ => Err(AppError::Domain(NOT_THIS_PASSPORT.to_owned())),
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
             SET name = $1, birth_date = $2, passport_expiry = $3, last_document_ref = $4 \
             WHERE branch_id = $5 AND id = $6",
        )
        .bind(&details.name)
        .bind(details.birth_date)
        .bind(details.passport_expiry)
        .bind(document_ref)
        .bind(branch_id.as_str())
        .bind(customer_id)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }
}

impl CustomerRepository for PgCustomerRepository {
    async fn branch_exists(&self, branch_id: &BranchId) -> AppResult<bool> {
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM branches WHERE id = $1")
            .bind(branch_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(exists.is_some())
    }

    async fn find(&self, branch_id: &BranchId, id: &str) -> AppResult<Option<Customer>> {
        let row: Option<CustomerDbRow> = sqlx::query_as(
            "SELECT id, branch_id, name, passport_no, nationality, birth_date, passport_expiry, mode, created_at, last_document_ref \
             FROM customers WHERE branch_id = $1 AND id = $2",
        )
        .bind(branch_id.as_str())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(map_customer_row))
    }

    async fn list_live(&self, branch_id: &BranchId) -> AppResult<Vec<Customer>> {
        let rows: Vec<CustomerDbRow> = sqlx::query_as(
            "SELECT id, branch_id, name, passport_no, nationality, birth_date, passport_expiry, mode, created_at, last_document_ref \
             FROM customers WHERE branch_id = $1 AND mode = 'LIVE' \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(branch_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(map_customer_row).collect())
    }
}
