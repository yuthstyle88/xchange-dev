//! Customer service.

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::customer::commands::reserve_customer::ReserveCustomer;
use crate::domain::customer::entities::customer::Customer;
use crate::domain::customer::entities::reservation::Reservation;
use crate::domain::customer::repositories::customer_repository::CustomerRepository;
use crate::domain::customer::value_objects::customer_details::CustomerDetails;
use crate::domain::customer::value_objects::mode::Mode;
use crate::infrastructure::customer::customer_repository::PgCustomerRepository;

pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const COUNTER_EXHAUSTED: &str = "ตัวนับรหัสไม่ถูกต้อง";

#[derive(Clone)]
pub struct CustomerService {
    branch_tx: BranchTx,
    repo: PgCustomerRepository,
}

impl CustomerService {
    pub fn new(branch_tx: BranchTx) -> Self {
        let repo = PgCustomerRepository::new(branch_tx.pool().clone());
        Self { branch_tx, repo }
    }

    pub fn branch_tx(&self) -> &BranchTx {
        &self.branch_tx
    }

    pub async fn reserve(
        &self,
        branch: &BranchId,
        cmd: ReserveCustomer,
        now: DateTime<Utc>,
    ) -> AppResult<Reservation> {
        if !self.repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }

        let details = CustomerDetails::parse(&cmd)?;

        let b = branch.clone();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    if let Some(existing_id) = PgCustomerRepository::find_existing_in_tx(
                        tx,
                        &b,
                        &details.passport_no,
                        &details.nationality,
                        details.mode,
                    )
                    .await?
                    {
                        return Ok(Reservation {
                            customer_id: existing_id,
                            existing: true,
                        });
                    }

                    let counter_key = match details.mode {
                        Mode::Live => "customer-LIVE",
                        Mode::Demo => "customer-DEMO",
                    };
                    let next_val = PgCustomerRepository::increment_counter_in_tx(tx, counter_key)
                        .await?
                        .ok_or_else(|| AppError::Domain(COUNTER_EXHAUSTED.to_owned()))?;

                    let customer_id = match details.mode {
                        Mode::Live => format!("CX-{next_val:08}"),
                        Mode::Demo => format!("TEST-CX-{next_val:08}"),
                    };

                    let created_at = Utc
                        .timestamp_millis_opt(now.timestamp_millis())
                        .single()
                        .unwrap_or(now);

                    let new_customer = Customer {
                        id: customer_id.clone(),
                        branch_id: b.clone(),
                        name: details.name,
                        passport_no: details.passport_no,
                        nationality: details.nationality,
                        birth_date: details.birth_date,
                        passport_expiry: details.passport_expiry,
                        mode: details.mode,
                        created_at,
                        last_document_ref: None,
                    };

                    PgCustomerRepository::insert_customer_in_tx(tx, &new_customer).await?;

                    Ok(Reservation {
                        customer_id,
                        existing: false,
                    })
                })
            })
            .await
    }

    pub async fn find(&self, branch: &BranchId, id: &str) -> AppResult<Option<Customer>> {
        self.repo.find(branch, id).await
    }

    pub async fn list_live(&self, branch: &BranchId) -> AppResult<Vec<Customer>> {
        if !self.repo.branch_exists(branch).await? {
            return Err(AppError::Domain(NO_SUCH_BRANCH.to_owned()));
        }
        self.repo.list_live(branch).await
    }

    pub async fn confirm_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
        details: &CustomerDetails,
    ) -> AppResult<()> {
        PgCustomerRepository::confirm_customer_in_tx(tx, branch, customer_id, details).await
    }

    pub async fn record_exchange_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        customer_id: &str,
        details: &CustomerDetails,
        document_ref: &str,
    ) -> AppResult<()> {
        PgCustomerRepository::record_exchange_in_tx(tx, branch, customer_id, details, document_ref)
            .await
    }
}
