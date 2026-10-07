//! Ticket #94 — a shift or movement row the server cannot decode is reported as different money
//! instead of as a fault. Written before the fix. Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! The readers of `PgShiftRepository` (`api/crates/xc-shift/src/infrastructure/shift/
//! shift_repository.rs`) substitute a default for a stored value they cannot decode: a movement
//! in a currency the register does not know is listed as baht, and such a balance row is
//! dropped, so its figures read as 0. The rule (#94, ticket #48 §7): a row that cannot be
//! decoded is a fault — an `AppError` other than `AppError::Domain` — never a substitute.
//!
//! The schema lets exactly one such value in through a plain write: `code` is `char(3)` with no
//! check against the register (`direction` and `status` have `CHECK` constraints; amounts are
//! `numeric(20,2)`). The tests write the shift and the movement through the service, then turn
//! the currency into `ZZZ` by hand, as a hand-edited row or a currency removed from the register
//! would leave it.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod shift_service_common;

use pos_error::{AppError, AppResult};
use serde_json::json;
use shift_service_common::{TestDb, at};
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift::commands::move_stock::MoveStock;
use xc_shift::domain::shift::commands::open_shift::OpenShift;

fn rid(label: &str, n: usize) -> String {
    format!("req-{label}-{n:04}-000000000000")
}

/// A branch with an open shift holding USD 100 and one movement of USD 20 in.
async fn a_day_with_dollars(db: &TestDb, label: &str) -> BranchId {
    let branch = db.branch(label).await;
    let service = db.service();
    let shift = service
        .open(
            &branch,
            OpenShift::from_payload(
                &json!({ "requestId": rid(label, 0), "opening": { "USD": "100" } }),
            )
            .expect("a well-formed open payload"),
            "opener",
            at(0, 0),
        )
        .await
        .expect("open the shift");
    service
        .move_stock(
            &branch,
            MoveStock::from_payload(&json!({
                "requestId": rid(label, 1), "shiftId": shift.id, "currency": "USD",
                "direction": "IN", "amount": "20", "reason": "test",
            }))
            .expect("a well-formed movement payload"),
            "mover",
            at(60, 0),
        )
        .await
        .expect("move the stock");
    branch
}

fn a_fault<T: std::fmt::Debug>(read: AppResult<T>, what: &str) {
    match read {
        Err(AppError::Domain(sentence)) => {
            panic!("{what}: a fault is not a refusal, got Domain({sentence:?})")
        }
        Err(_) => {}
        Ok(value) => panic!("{what}: an undecodable row was read as {value:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_movement_in_a_currency_the_register_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let branch = a_day_with_dollars(&db, "move").await;
    sqlx::query("UPDATE stock_movements SET code = 'ZZZ' WHERE branch_id = $1")
        .bind(branch.as_str())
        .execute(&db.pool)
        .await
        .expect("turn the movement's currency into ZZZ");

    a_fault(db.service().movements(&branch).await, "movements");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_balance_in_a_currency_the_register_does_not_know_is_a_fault() {
    let db = TestDb::start().await;
    let branch = a_day_with_dollars(&db, "balance").await;
    sqlx::query(
        "UPDATE shift_balances SET code = 'ZZZ' WHERE code = 'USD' AND shift_id IN \
         (SELECT id FROM shifts WHERE branch_id = $1)",
    )
    .bind(branch.as_str())
    .execute(&db.pool)
    .await
    .expect("turn the balance's currency into ZZZ");

    a_fault(db.service().shifts(&branch).await, "shifts");
    a_fault(db.service().active(&branch).await, "active");
    db.drop_schema().await;
}

/// Holds before the fix and after it: rows that decode are read as written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rows_that_decode_are_read_as_written() {
    let db = TestDb::start().await;
    let branch = a_day_with_dollars(&db, "fine").await;
    let service = db.service();
    let movements = service.movements(&branch).await.expect("movements");
    assert_eq!(movements.len(), 1);
    assert_eq!(movements[0].currency.as_str(), "USD");
    assert_eq!(movements[0].amount.to_string(), "20.00");
    let shift = service
        .active(&branch)
        .await
        .expect("active")
        .expect("a shift is open");
    assert_eq!(
        shift
            .expected
            .get(xc_kernel::money::CurrencyCode::parse("USD").unwrap())
            .to_string(),
        "120.00"
    );
    db.drop_schema().await;
}
