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
//!
//! CA-10 (#626, #2955) restated the setup: v2's `ShiftService::{open, move_stock}` and the per-branch `active` are
//! deleted, so the shift and its movement are seeded straight into the tables, and the open shift is read by its id.

mod shift_service_common;

use shift_service_common::{TestDb, at};
use xc_kernel::ids::BranchId;

fn rid(label: &str, n: usize) -> String {
    format!("req-{label}-{n:04}-000000000000")
}

/// A branch with an open shift holding USD 100 and one movement of USD 20 in. Returns the branch and the shift.
async fn a_day_with_dollars(db: &TestDb, label: &str) -> (BranchId, String) {
    let branch = db.branch(label).await;
    let shift = db
        .seed_open_shift(
            &branch,
            "opener",
            &rid(label, 0),
            &[("USD", "100")],
            at(0, 0),
        )
        .await;
    db.balances.seed_movement(&shift, "USD", "IN", "20");
    (branch, shift)
}

/// Holds before the fix and after it: rows that decode are read as written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rows_that_decode_are_read_as_written() {
    let db = TestDb::start().await;
    let (branch, id) = a_day_with_dollars(&db, "fine").await;
    let service = db.service();
    let shift = service
        .shift(&branch, &id)
        .await
        .expect("shift")
        .expect("the shift exists");
    assert_eq!(
        shift
            .expected
            .get(xc_kernel::money::CurrencyCode::parse("USD").unwrap())
            .to_string(),
        "120.00"
    );
    db.drop_schema().await;
}
