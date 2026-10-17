//! Specification tests for `PgShiftRepository` row-to-`Shift` mapping and `SELECT` clauses (#1888).
//!
//! CA-10 (#626, #2955) restated these on SQL-seeded shifts: v2's `ShiftService::{open, close}` and the per-branch
//! `ShiftRepository::{active, active_in}` are deleted. The reads of an OPEN shift are now the per-teller ones
//! (`open_of_teller`, `open_of_teller_in`, CA-10p #2916). `a_count_opened_shift_is_not_the_active_one` is retired:
//! its subject was `active`'s `opened_by_count IS NULL` filter, which goes with `active` and with the column (M3).

mod shift_service_common;

use pos_error::{AppError, AppResult};
use shift_service_common::{TestDb, at, code, money};
use xc_kernel::money::MoneyDelta;
use xc_shift::domain::shift::entities::shift::ShiftStatus;
use xc_shift::domain::shift::repositories::shift_repository::ShiftRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

fn assert_internal_fault<T: std::fmt::Debug>(res: AppResult<T>, column: &str, bad_value: &str) {
    match res {
        Err(AppError::Internal(msg)) => {
            assert!(
                msg.contains(column) && msg.contains(bad_value),
                "expected internal fault mentioning `{column}` and `{bad_value}`, got: {msg}"
            );
        }
        other => panic!("expected AppError::Internal for {column}={bad_value}, got: {other:?}"),
    }
}

#[tokio::test]
async fn the_six_reads_return_the_same_shift() {
    let db = TestDb::start().await;
    let branch = db.branch("MAIN").await;
    let id = db
        .seed_open_shift(
            &branch,
            "opener",
            "req-open-0001-000000000000",
            &[("USD", "100.00")],
            at(0, 0),
        )
        .await;
    let teller = db.teller("opener").await;

    let repo = PgShiftRepository::new(db.pool.clone());
    let usd = code("USD");

    // Verify the 5 reads applicable before any close_request_id is set on the row.
    let from_active = repo
        .open_of_teller(&teller)
        .await
        .unwrap()
        .expect("the teller's open shift exists");
    let from_shifts = repo.shifts(&branch).await.unwrap();
    assert_eq!(from_shifts.len(), 1);
    let from_shift_by_id = repo
        .shift(&branch, &id)
        .await
        .unwrap()
        .expect("shift by id exists");
    let mut tx = db.pool.begin().await.unwrap();
    let from_open_req = repo
        .find_shift_by_open_request_id(&mut tx, &branch, "req-open-0001-000000000000")
        .await
        .unwrap()
        .expect("shift by open_request_id exists");
    let from_active_in = repo
        .open_of_teller_in(&mut tx, &teller)
        .await
        .unwrap()
        .expect("open_of_teller_in shift exists");
    tx.commit().await.unwrap();

    assert_eq!(from_active.status, ShiftStatus::Open);
    assert_eq!(from_active.counted, None);
    assert_eq!(from_active.difference, None);
    assert_eq!(from_active.close_request_id, None);
    assert_eq!(from_active.opening.get(usd), money("100.00"));
    assert_eq!(from_active.expected.get(usd), money("100.00"));
    assert_eq!(from_active.id, id);
    assert_eq!(from_active.opened_at, at(0, 0));
    assert_eq!(from_active.open_request_id, "req-open-0001-000000000000");

    assert_eq!(from_shifts[0], from_active);
    assert_eq!(from_shift_by_id, from_active);
    assert_eq!(from_open_req, from_active);
    assert_eq!(from_active_in, from_active);

    // Stamp close_request_id on the open row so all six read methods can read the exact same row.
    sqlx::query("UPDATE shifts SET close_request_id = 'req-close-0001-000000000000' WHERE id = $1")
        .bind(&id)
        .execute(&db.pool)
        .await
        .unwrap();

    let s_active = repo.open_of_teller(&teller).await.unwrap().unwrap();
    let s_shifts = repo.shifts(&branch).await.unwrap();
    let s_by_id = repo.shift(&branch, &id).await.unwrap().unwrap();
    let mut tx = db.pool.begin().await.unwrap();
    let s_by_open = repo
        .find_shift_by_open_request_id(&mut tx, &branch, "req-open-0001-000000000000")
        .await
        .unwrap()
        .unwrap();
    let s_by_close = repo
        .find_shift_by_close_request_id(&mut tx, &branch, "req-close-0001-000000000000")
        .await
        .unwrap()
        .unwrap();
    let s_active_in = repo
        .open_of_teller_in(&mut tx, &teller)
        .await
        .unwrap()
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(
        s_active.close_request_id.as_deref(),
        Some("req-close-0001-000000000000")
    );
    assert_eq!(s_shifts, vec![s_active.clone()]);
    assert_eq!(s_by_id, s_active);
    assert_eq!(s_by_open, s_active);
    assert_eq!(s_by_close, s_active);
    assert_eq!(s_active_in, s_active);

    db.drop_schema().await;
}

#[tokio::test]
async fn a_closed_shift_reads_the_same_by_id_and_in_the_list() {
    let db = TestDb::start().await;
    let branch = db.branch("MAIN").await;
    let opened = db
        .seed_open_shift(
            &branch,
            "opener",
            "req-open-0001-000000000000",
            &[("USD", "100.00")],
            at(0, 0),
        )
        .await;
    db.seed_close(
        &opened,
        "b",
        "req-close-0001-000000000000",
        &[("USD", "95.00")],
        "short 5",
        at(0, 30),
    )
    .await;

    let repo = PgShiftRepository::new(db.pool.clone());
    let usd = code("USD");

    let by_id = repo
        .shift(&branch, &opened)
        .await
        .unwrap()
        .expect("closed shift by id");
    let list = repo.shifts(&branch).await.unwrap();
    assert_eq!(list.len(), 1);

    let mut tx = db.pool.begin().await.unwrap();
    let by_close_req = repo
        .find_shift_by_close_request_id(&mut tx, &branch, "req-close-0001-000000000000")
        .await
        .unwrap()
        .expect("closed shift by close_request_id");
    tx.commit().await.unwrap();

    assert_eq!(by_id.status, ShiftStatus::Closed);
    let counted = by_id
        .counted
        .as_ref()
        .expect("counted is Some on closed shift");
    assert_eq!(counted.get(usd), money("95.00"));
    let diff = by_id
        .difference
        .as_ref()
        .expect("difference is Some on closed shift");
    assert_eq!(diff.get(usd), MoneyDelta::ZERO.minus(money("5.00")));
    assert_eq!(by_id.note, "short 5");
    assert_eq!(
        by_id.close_request_id.as_deref(),
        Some("req-close-0001-000000000000")
    );

    assert_eq!(by_id.closed_at, Some(at(0, 30)));
    assert_eq!(by_id.closed_by.as_deref(), Some("b"));
    assert_eq!(list[0], by_id);
    assert_eq!(by_close_req, by_id);

    db.drop_schema().await;
}

#[tokio::test]
async fn an_undecodable_status_is_a_fault_on_every_read() {
    let db = TestDb::start().await;
    let branch = db.branch("MAIN").await;
    let s = db
        .seed_open_shift(
            &branch,
            "opener",
            "req-open-broken-000000000000",
            &[("USD", "50.00")],
            at(0, 0),
        )
        .await;

    sqlx::query("ALTER TABLE shifts DROP CONSTRAINT shifts_status_check")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE shifts SET status = 'BROKEN', close_request_id = 'req-close-broken-000000000000' WHERE id = $1",
    )
    .bind(&s)
    .execute(&db.pool)
    .await
    .unwrap();

    let repo = PgShiftRepository::new(db.pool.clone());

    assert_internal_fault(repo.shifts(&branch).await, "shifts.status", "BROKEN");
    assert_internal_fault(repo.shift(&branch, &s).await, "shifts.status", "BROKEN");

    let mut tx = db.pool.begin().await.unwrap();
    assert_internal_fault(
        repo.find_shift_by_open_request_id(&mut tx, &branch, "req-open-broken-000000000000")
            .await,
        "shifts.status",
        "BROKEN",
    );
    assert_internal_fault(
        repo.find_shift_by_close_request_id(&mut tx, &branch, "req-close-broken-000000000000")
            .await,
        "shifts.status",
        "BROKEN",
    );
    drop(tx);

    db.drop_schema().await;
}
