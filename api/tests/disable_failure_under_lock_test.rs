//! Ticket #202 — nothing pinned what a disable answers when a database step
//! fails under the branch lock. Written after the fix; this test only pins it.
//!
//! `DisableEmployee::run` asks custody and closes the account inside one
//! `BranchTx::run` of the employee's branch (#157). This test holds the failure
//! half of that promise: when the closing's own write to the database fails
//! inside the lock, the workflow answers an error — never `Closed` — and the
//! transaction is rolled back, so the account stays as it was, active with its
//! version untouched, and the branch's version does not move (#157 §8: "A
//! database error in either step → the route's `failure(e)` as today; the
//! transaction is rolled back").
//!
//! The failure is made the way the ticket offers: the audit table is dropped in
//! the test's own schema once the employee exists, so the closing's `UPDATE`
//! succeeds inside the transaction and the audit `INSERT` after it fails on the
//! missing table. No repository is doubled: the real `AccountService` runs
//! against the real PostgreSQL these tests need (`TEST_DATABASE_URL`, a
//! database whose name ends in `_test`); without it they FAIL, none skips. No
//! sleep orders anything.
//!
//! Red with the mutant of ticket #202 applied (`Err(_) => Ok(DisableOutcome::
//! Closed)` at the end of `run`): the workflow would answer `Closed` for a
//! closing that did not happen.

mod auth_http_common;
mod common;

use std::sync::Arc;
use std::time::SystemTime;

use auth_http_common::Desk;
use pos_infra::transactions::Transactor;
use serde_json::json;
use xc_auth::application::account::service::{AccountService, EmployeesAnswer};
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xchange::application::workflows::disable_employee::DisableEmployee;

const MAIN: &str = "MAIN";

fn branch_tx(desk: &Desk) -> BranchTx {
    BranchTx::new(Transactor::new(desk.pool.clone()))
}

/// The service the route runs, on the desk's pool: the real repository, not a double.
fn accounts(desk: &Desk) -> AccountService<PgAccountRepository> {
    AccountService::new(
        Arc::new(PgAccountRepository::new(desk.pool.clone())),
        PasswordHasher::with_cost(8, 1, 1),
    )
}

async fn active_and_version(desk: &Desk, id: &str) -> (bool, i32) {
    sqlx::query_as("SELECT active, version FROM employees WHERE id = $1")
        .bind(id)
        .fetch_one(&desk.pool)
        .await
        .expect("the employee row")
}

/// A database failure under the branch lock is answered as an error, never as a
/// closing, and the transaction it happened in is rolled back.
#[actix_web::test]
async fn a_database_failure_under_the_lock_is_an_error_and_the_account_stays_active() {
    let desk = Desk::start().await;
    let service = accounts(&desk);
    match service
        .employees(
            "create",
            &json!({ "username": "ann", "name": "คุณ ann", "role": "FRONT", "branchId": MAIN,
                     "password": "the-first-password" }),
            "owner",
            SystemTime::now().into(),
        )
        .await
    {
        Ok(EmployeesAnswer::Saved) => {}
        other => panic!("create the employee: {other:?}"),
    }
    let id = desk.employee_id("ann").await;
    let before = active_and_version(&desk, &id).await;
    assert!(before.0, "the employee starts active");
    let version_before = branch_tx(&desk)
        .version(&BranchId::new(MAIN))
        .await
        .expect("read the branch's version");

    // The sabotage: the closing's audit write fails on a table that is gone.
    sqlx::query("DROP TABLE employee_audit")
        .execute(&desk.pool)
        .await
        .expect("drop the audit table in the test's schema");

    let ran = DisableEmployee::new(branch_tx(&desk))
        .run(
            &service,
            None, // nobody holds cash, as for the route
            &BranchId::new(MAIN),
            &id,
            "owner",
            SystemTime::now().into(),
        )
        .await;

    match ran {
        Ok(outcome) => panic!(
            "a database failure under the lock answered {outcome:?}, not an error — \
             the route would say saved for a closing that did not happen"
        ),
        Err(error) => assert!(
            matches!(error, pos_error::AppError::Database(_)),
            "the failure inside the lock is a database error: {error:?}"
        ),
    }
    assert_eq!(
        active_and_version(&desk, &id).await,
        before,
        "the rolled-back transaction closes nothing"
    );
    assert_eq!(
        branch_tx(&desk)
            .version(&BranchId::new(MAIN))
            .await
            .expect("read the branch's version"),
        version_before,
        "a write that did not happen does not move the branch's version"
    );
    desk.drop_schema().await;
}
