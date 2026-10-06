//! The real `SnapshotSources`: what the snapshot shows that its builder has no
//! service for, read through the services that own it.
//!
//! - the exchanges of a branch come from the exchange context's
//!   `TransactionService` — the LIVE ones only, in the order they were saved;
//! - the people of a branch come from the accounts' employee list, disabled
//!   accounts included: a custody row names its sender long after the account
//!   is closed;
//! - nobody is a member: the members decision (D5) is not taken, so there is no
//!   table to read. When it lands, `members` is the one method to fill in.
//!
//! No SQL here: both services keep theirs behind their own repositories.

use chrono::Utc;
use pos_error::AppError;
use serde_json::json;
use xc_auth::application::account::service::{AccountService, EmployeesAnswer};
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_kernel::ids::BranchId;

use crate::application::compat::snapshot_sources::{
    MemberMark, SnapshotSources, SnapshotTransaction, SourceFuture,
};
use crate::application::compat::v2_json::{People, PersonLabel};

/// Who the employee list is read as. Reading writes no audit row; the name
/// only says where the question came from.
const READER: &str = "snapshot";

/// `SnapshotSources` over the services on main.
#[derive(Clone)]
pub struct ServiceSnapshotSources {
    transactions: TransactionService,
    accounts: AccountService<PgAccountRepository>,
}

impl ServiceSnapshotSources {
    pub fn new(
        transactions: TransactionService,
        accounts: AccountService<PgAccountRepository>,
    ) -> Self {
        Self {
            transactions,
            accounts,
        }
    }
}

/// One saved exchange as the snapshot shows it. `staff` is what the save
/// recorded as `staff` — the dispatcher sets it to the caller's username before
/// it saves, as v2's routes do — and the method is the stored one: the builder
/// prints the old `SCB Transfer` as `Bank Transfer`.
fn snapshot_transaction(saved: Transaction) -> SnapshotTransaction {
    SnapshotTransaction {
        document_ref: saved.document_ref,
        recorded_at: saved.recorded_at,
        value_date: saved.value_date,
        side: saved.side,
        currency: saved.currency,
        amount: saved.amount,
        rate: saved.rate,
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
            Ok(saved.into_iter().map(snapshot_transaction).collect())
        })
    }

    fn members<'a>(&'a self, _branch: &'a BranchId) -> SourceFuture<'a, Vec<MemberMark>> {
        // decision D5 is open: there is no members table yet
        Box::pin(async { Ok(Vec::new()) })
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
}
