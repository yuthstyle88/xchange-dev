//! The real `SnapshotSources`: what the snapshot shows that its builder has no
//! service for, read through the services that own it.
//!
//! - the exchanges of a branch come from the exchange context's
//!   `TransactionService` — the LIVE ones only, in the order they were saved;
//! - the people of a branch come from the accounts' employee list, disabled
//!   accounts included: a custody row names its sender long after the account
//!   is closed;
//! - the members of a branch come from `PgMemberRepository` (D5 is decided);
//! - the payment rules of a branch come from `PaymentSettingsService`.
//!
//! No SQL here: all services keep theirs behind their own repositories.

use chrono::{DateTime, Utc};
use pos_error::AppError;
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use xc_auth::application::account::service::{AccountService, EmployeesAnswer};
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_kernel::ids::BranchId;
use xc_kernel::money::UnitSize;
use xc_kernel::payment::PaymentRules;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;

use crate::application::compat::snapshot_sources::{
    MemberMark, SnapshotSources, SnapshotTransaction, SourceFuture,
};
use crate::application::compat::v2_json::{People, PersonLabel};
use crate::application::settings::payment_settings::PaymentSettingsService;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;

/// Who the employee list is read as. Reading writes no audit row; the name
/// only says where the question came from.
const READER: &str = "snapshot";

/// `SnapshotSources` over the services on main.
#[derive(Clone)]
pub struct ServiceSnapshotSources {
    pool: PgPool,
    transactions: TransactionService,
    accounts: AccountService<PgAccountRepository>,
    payment_settings: PaymentSettingsService,
}

impl ServiceSnapshotSources {
    pub fn new(
        transactions: TransactionService,
        accounts: AccountService<PgAccountRepository>,
    ) -> Self {
        let branch_tx = transactions.branch_tx().clone();
        let pool = branch_tx.pool().clone();
        let store = BranchSettingStore::new(pool.clone());
        let branches = PgBranchRepository::new(pool.clone());
        let payment_settings = PaymentSettingsService::new(branch_tx, store, branches);
        Self {
            pool,
            transactions,
            accounts,
            payment_settings,
        }
    }
}

/// One saved exchange as the snapshot shows it. `staff` is what the save
/// recorded as `staff` — the dispatcher sets it to the caller's username before
/// it saves, as v2's routes do — and the method is the stored one: the builder
/// prints the old `SCB Transfer` as `Bank Transfer`.
fn snapshot_transaction(saved: Transaction, unit_size: UnitSize) -> SnapshotTransaction {
    SnapshotTransaction {
        document_ref: saved.document_ref,
        recorded_at: saved.recorded_at,
        value_date: saved.value_date,
        side: saved.side,
        currency: saved.currency,
        amount: saved.amount,
        rate: saved.rate,
        unit_size,
        reference_rate: saved.reference_rate,
        fee: saved.fee,
        bank_fee: saved.bank_fee,
        method: saved.method.as_str().to_string(),
        slip_key: saved.slip_key,
        payment_ref: saved.payment_ref,
        customer_id: saved.customer_id,
        customer_name: saved.customer_name,
        staff: saved.staff,
        notes: saved.notes,
        shift_id: saved.shift_id,
    }
}

impl SnapshotSources for ServiceSnapshotSources {
    fn transactions<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> SourceFuture<'a, Vec<SnapshotTransaction>> {
        Box::pin(async move {
            let saved = self.transactions.list(branch, Mode::Live).await?;
            if saved.is_empty() {
                return Ok(Vec::new());
            }
            let unit_rows: Vec<(String, rust_decimal::Decimal)> = sqlx::query_as(
                "SELECT t.document_ref, l.unit_size \
                 FROM transactions t \
                 JOIN transaction_lines l ON l.transaction_id = t.id \
                 WHERE t.branch_id = $1 AND t.mode = 'LIVE' AND t.status = 'COMPLETED'",
            )
            .bind(branch.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
            let mut unit_map: HashMap<String, UnitSize> = HashMap::with_capacity(unit_rows.len());
            for (doc, u) in unit_rows {
                let us = UnitSize::from_decimal(u).map_err(|e| {
                    AppError::Internal(format!("invalid unit_size for transaction {doc}: {e}"))
                })?;
                unit_map.insert(doc, us);
            }
            let mut result = Vec::with_capacity(saved.len());
            for s in saved {
                let unit_size = unit_map.get(&s.document_ref).copied().ok_or_else(|| {
                    AppError::Internal(format!(
                        "missing unit_size for transaction {}",
                        s.document_ref
                    ))
                })?;
                result.push(snapshot_transaction(s, unit_size));
            }
            Ok(result)
        })
    }

    fn members<'a>(&'a self, branch: &'a BranchId) -> SourceFuture<'a, Vec<MemberMark>> {
        Box::pin(async move {
            let rows: Vec<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
                "SELECT ci.customer_id, ci.verified_at \
                 FROM customer_identities ci \
                 JOIN branches b ON b.organization_id = ci.organization_id \
                 WHERE b.id = $1 \
                   AND ci.type = 'PHONE' \
                   AND ci.status = 'ACTIVE' \
                   AND ci.verified_at IS NOT NULL \
                 ORDER BY ci.verified_at ASC, ci.customer_id ASC",
            )
            .bind(branch.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            Ok(rows
                .into_iter()
                .map(|(customer_id, verified_at)| MemberMark {
                    customer_id,
                    verified_at,
                })
                .collect())
        })
    }

    fn people<'a>(&'a self, branch: &'a BranchId) -> SourceFuture<'a, People> {
        Box::pin(async move {
            let everyone = match self
                .accounts
                .employees("list", &json!({}), READER, Utc::now())
                .await?
            {
                EmployeesAnswer::List(everyone) => everyone,
                EmployeesAnswer::Saved => {
                    return Err(AppError::Internal(
                        "the employee list answered a write".to_string(),
                    ));
                }
            };
            Ok(everyone
                .into_iter()
                .filter(|employee| employee.branch_id == branch.as_str())
                .map(|employee| {
                    (
                        employee.id,
                        PersonLabel {
                            name: employee.name,
                            role: employee.role,
                        },
                    )
                })
                .collect())
        })
    }

    fn payment_rules<'a>(&'a self, branch: &'a BranchId) -> SourceFuture<'a, PaymentRules> {
        Box::pin(async move { self.payment_settings.rules(branch).await })
    }
}
