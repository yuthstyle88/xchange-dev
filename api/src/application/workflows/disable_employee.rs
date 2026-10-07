//! `DisableEmployee`: the owner closes an employee's account under the lock of
//! the employee's branch.
//!
//! Custody's commands that make somebody the custodian or a party of a request
//! run under `BranchTx::run` of their branch. The question "does this employee
//! hold cash or have work pending" and the closing of the account are asked and
//! done in one `BranchTx::run` of the same branch, so no such command can come
//! between them (#157).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::account::repositories::account_repository::AccountRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::presentation::http::employees::EmployeeCustody;

/// What a disable came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableOutcome {
    /// The account is closed, with its audit line.
    Closed,
    /// The employee holds cash or has work pending; nothing was written.
    HoldsCash,
}

/// Carries a refusal out of the locked closure, so that `BranchTx::run` rolls
/// the transaction back; turned into `DisableOutcome::HoldsCash` by `run` and
/// never seen outside this file.
const HOLDS_CASH_MARK: &str = "disable_employee: the employee holds cash or has work pending";

/// Asks custody and closes the account under the employee's branch lock.
#[derive(Clone)]
pub struct DisableEmployee {
    branch_tx: BranchTx,
}

impl DisableEmployee {
    pub fn new(branch_tx: BranchTx) -> Self {
        Self { branch_tx }
    }

    /// In one `BranchTx::run` of `branch` (the employee's): custody is asked
    /// through the transaction, then the account is closed through it. A
    /// refusal rolls the transaction back. `custody` `None` = nobody holds
    /// cash, as for the route. `accounts` answers `NOT_FOUND` for an id that
    /// names nobody.
    pub async fn run<R: AccountRepository>(
        &self,
        accounts: &AccountService<R>,
        custody: Option<Arc<dyn EmployeeCustody>>,
        branch: &BranchId,
        employee_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<DisableOutcome> {
        let accounts = accounts.clone();
        let (lock, employee_id, actor) = (branch.clone(), employee_id.to_owned(), actor.to_owned());
        let ran = self
            .branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    if let Some(custody) = custody
                        && custody
                            .holds_cash_or_pending_in(tx, &lock, &employee_id)
                            .await?
                    {
                        return Err(AppError::Conflict(HOLDS_CASH_MARK.to_owned()));
                    }
                    accounts
                        .disable_employee_in(tx, &employee_id, now, &actor)
                        .await
                })
            })
            .await;
        match ran {
            Ok(()) => Ok(DisableOutcome::Closed),
            Err(AppError::Conflict(mark)) if mark == HOLDS_CASH_MARK => {
                Ok(DisableOutcome::HoldsCash)
            }
            Err(e) => Err(e),
        }
    }
}
