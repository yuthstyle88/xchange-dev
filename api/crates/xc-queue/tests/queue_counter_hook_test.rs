//! #1128 (QU-A3) — `CounterService` is the shift's `ShiftLifecycleHook`: a shift opened by a
//! teller staffs that teller's counter, and its close releases it. Written before the
//! implementation by claude/dell-claude-6 on the order of the lane controller claude/ad05bf8d. Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! The shift side (`ShiftService` calls the hook in the shift's transaction, skips it with no
//! session user, and rolls back when it fails) is `xc-shift/tests/shift_lifecycle_hook_test.rs`.
//! These tests drive `CounterService` only through `dyn ShiftLifecycleHook`, the way
//! `ShiftService` will see it.
//!
//! Signatures this test assumes (`[required]` once pushed):
//! - `impl xc_kernel::shift_hook::ShiftLifecycleHook for CounterService` (in
//!   `xc-queue/src/application/counter_hook.rs`);
//! - `on_opened(tx, &BranchId, teller_user_id: &str, now)` / `on_closed(...)`
//!   -> `BoxFuture<AppResult<()>>`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use queue_common::*;
use queue_desk_common::*;
use xc_kernel::ids::BranchId;
use xc_kernel::shift_hook::ShiftLifecycleHook;

async fn open_main() -> (TestDb, Queue) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let q = queue(&db);
    (db, q)
}

fn hook_of(q: &Queue) -> Arc<dyn ShiftLifecycleHook> {
    Arc::new(q.counters.clone())
}

/// Runs the hook's `on_opened` for `teller` in one MAIN branch transaction, as `ShiftService`
/// will.
async fn opened(db: &TestDb, hook: &Arc<dyn ShiftLifecycleHook>, teller: &str, now: DateTime<Utc>) {
    let (h, t) = (hook.clone(), teller.to_owned());
    let b = main_branch();
    let done: AppResult<()> = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move { h.on_opened(tx, &b, &t, now).await })
        })
        .await;
    done.unwrap_or_else(|e| panic!("on_opened for {teller}: {e:?}"));
}

async fn closed(db: &TestDb, hook: &Arc<dyn ShiftLifecycleHook>, teller: &str, now: DateTime<Utc>) {
    let (h, t) = (hook.clone(), teller.to_owned());
    let b = main_branch();
    let done: AppResult<()> = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move { h.on_closed(tx, &b, &t, now).await })
        })
        .await;
    done.unwrap_or_else(|e| panic!("on_closed for {teller}: {e:?}"));
}

/// `teller status` of every counter of the branch, by teller.
async fn counters(db: &TestDb, branch: &str) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT teller_id::text || ' ' || status FROM queue_counters WHERE branch_id = '{branch}' \
         ORDER BY teller_id"
    )))
    .fetch_all(&db.pool)
    .await
    .expect("read the counters");
    rows.into_iter().map(|(r,)| r).collect()
}

// (a)
#[tokio::test]
async fn the_hook_told_a_shift_opened_staffs_that_tellers_counter() {
    let (db, q) = open_main().await;
    let hook = hook_of(&q);
    let a = db.add_user("Teller A").await;
    assert_eq!(counters(&db, "MAIN").await, Vec::<String>::new());
    assert_eq!(active_counters(&db, "MAIN").await, 0);

    opened(&db, &hook, &a, at(2, 0)).await;

    assert_eq!(
        counters(&db, "MAIN").await,
        vec![format!("{a} ACTIVE")],
        "on_opened must give the teller an ACTIVE counter"
    );
    assert_eq!(
        active_counters(&db, "MAIN").await,
        1,
        "and the branch status must count it"
    );
    db.finish().await;
}

// (b)
#[tokio::test]
async fn the_hook_told_a_shift_closed_releases_that_tellers_counter_only() {
    let (db, q) = open_main().await;
    let hook = hook_of(&q);
    let a = db.add_user("Teller A").await;
    let b = db.add_user("Teller B").await;
    opened(&db, &hook, &a, at(2, 0)).await;
    opened(&db, &hook, &b, at(2, 5)).await;
    assert_eq!(active_counters(&db, "MAIN").await, 2);

    closed(&db, &hook, &a, at(3, 0)).await;

    assert_eq!(
        counters(&db, "MAIN").await,
        vec![format!("{b} ACTIVE")],
        "on_closed must remove the closing teller's counter and leave the other"
    );
    assert_eq!(active_counters(&db, "MAIN").await, 1);
    db.finish().await;
}

// (b), a counter the teller paused is released as well
#[tokio::test]
async fn the_hook_told_a_shift_closed_releases_a_paused_counter() {
    let (db, q) = open_main().await;
    let hook = hook_of(&q);
    let a = db.add_user("Teller A").await;
    opened(&db, &hook, &a, at(2, 0)).await;
    q.counters
        .pause(&teller(&a, "MAIN"), &BranchId::new("MAIN"), at(2, 30))
        .await
        .expect("pause the counter");

    closed(&db, &hook, &a, at(3, 0)).await;

    assert_eq!(counters(&db, "MAIN").await, Vec::<String>::new());
    assert_eq!(active_counters(&db, "MAIN").await, 0);
    db.finish().await;
}
