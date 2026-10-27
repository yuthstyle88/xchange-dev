//! Written before the implementation (ticket #5220). Do not edit, delete or weaken.
//!
//! Specification of `CustodyPages::attendance_since(branch, only, since, before, limit)`:
//! the rows `attendance` answers (same branch, `only`, `before` and `limit` rules, newest
//! first by `position`) that satisfy one of two conditions:
//! - `clockIn >= since` (inclusive), or
//! - the row has no `clockOut` (absent or JSON null): the person is still clocked in.
//!
//! Rows P1–P8 of the ticket's behaviour table, one test per row, named with the row id.
//! `attendance` itself is unchanged and stays pinned by `custody_pages_test.rs`.
//!
//! The rows are written through `CustodyRepository::store_state_in` (which stores an open
//! row's `clockOut` as JSON null), except where a row must have no `clockOut` key at all:
//! that one is a plain insert in the JSON shape `custody_repository.rs` writes
//! (`{"role","clockIn","clockOut"}`).
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they FAIL; none skips.

mod shift_service_common;

use serde_json::json;
use shift_service_common::{TestDb, unique, within};
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::repositories::custody_pages::CustodyPages;
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::infrastructure::custody::custody_pages::PgCustodyPages;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;

/// `since` = S: 2026-10-17 03:00:00 UTC in epoch milliseconds.
const S: i64 = 1_792_206_000_000;
const MS: i64 = 1;
const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

fn since() -> CustodyTime {
    CustodyTime::from_epoch_millis(S)
}

/// A row of `employee`, clocked in at S + `clock_in` and out at S + `clock_out` (if any).
fn row(id: &str, employee: &str, clock_in: i64, clock_out: Option<i64>) -> Attendance {
    Attendance {
        id: id.to_string(),
        employee_id: employee.to_string(),
        role: CustodyRole::Front,
        clock_in: CustodyTime::from_epoch_millis(S + clock_in),
        clock_out: clock_out.map(|o| CustodyTime::from_epoch_millis(S + o)),
    }
}

fn ids(rows: &[Attendance]) -> Vec<String> {
    rows.iter().map(|r| r.id.clone()).collect()
}

/// Writes `rows` as the branch's attendance, position i = the i-th, through the repository.
async fn store(pool: &PgPool, branch: &BranchId, rows: Vec<Attendance>) {
    let after = CustodyState {
        phase: Phase::Open,
        attendance: rows,
        ..CustodyState::initial()
    };
    let repo = PgCustodyRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    repo.store_state_in(&mut tx, branch, &CustodyState::initial(), &after)
        .await
        .expect("store the seed through CustodyRepository::store_state_in");
    tx.commit().await.expect("commit the seed");
}

fn pages(db: &TestDb) -> PgCustodyPages {
    PgCustodyPages::new(db.pool.clone())
}

async fn since_all(db: &TestDb, branch: &BranchId) -> Vec<Attendance> {
    pages(db)
        .attendance_since(branch, None, since(), None, 50)
        .await
        .expect("attendance_since")
}

// ── P1 ──────────────────────────────────────────────────────────────────────

/// P1: S + 1 h → S + 2 h is kept.
#[tokio::test]
async fn p1_a_row_clocked_in_after_since_and_closed_is_kept() {
    within("P1", async {
        let db = TestDb::start().await;
        let branch = db.branch("p1").await;
        let tag = unique("p1");
        let kept = row(&format!("{tag}-kept"), "e1", HOUR, Some(2 * HOUR));
        store(&db.pool, &branch, vec![kept.clone()]).await;

        let got = since_all(&db, &branch).await;
        assert!(got == vec![kept], "P1 kept; got {:?}", ids(&got));
    })
    .await;
}

// ── P2 ──────────────────────────────────────────────────────────────────────

/// P2: S − 1 h → S − 30 min is dropped (in and out both before S).
#[tokio::test]
async fn p2_a_row_clocked_in_and_out_before_since_is_dropped() {
    within("P2", async {
        let db = TestDb::start().await;
        let branch = db.branch("p2").await;
        let tag = unique("p2");
        let dropped = row(&format!("{tag}-dropped"), "e1", -HOUR, Some(-30 * MINUTE));
        let kept = row(&format!("{tag}-kept"), "e1", HOUR, Some(2 * HOUR));
        store(&db.pool, &branch, vec![dropped, kept.clone()]).await;

        let got = since_all(&db, &branch).await;
        assert!(
            got == vec![kept],
            "P2 dropped, only the kept row; got {:?}",
            ids(&got)
        );
    })
    .await;
}

// ── P3 ──────────────────────────────────────────────────────────────────────

/// P3: S − 1 day → none is kept: still clocked in. Its `clockOut` is JSON null (the way
/// the repository stores an open row) on one row and absent from `data` on the other.
#[tokio::test]
async fn p3_a_row_clocked_in_a_day_before_since_and_still_open_is_kept() {
    within("P3", async {
        let db = TestDb::start().await;
        let branch = db.branch("p3").await;
        let tag = unique("p3");
        let closed = row(&format!("{tag}-closed"), "e1", -DAY, Some(-DAY + HOUR));
        let null_out = row(&format!("{tag}-null-out"), "e2", -DAY, None);
        store(&db.pool, &branch, vec![closed, null_out.clone()]).await;

        let absent_out = row(&format!("{tag}-absent-out"), "e3", -DAY, None);
        sqlx::query(
            "INSERT INTO attendance (id, branch_id, position, employee_id, data) \
             VALUES ($1, $2, 2, $3, $4)",
        )
        .bind(&absent_out.id)
        .bind(branch.as_str())
        .bind(&absent_out.employee_id)
        .bind(json!({ "role": "FRONT", "clockIn": S - DAY }))
        .execute(&db.pool)
        .await
        .expect("insert an attendance row without a clockOut key");

        let got = since_all(&db, &branch).await;
        assert!(
            got == vec![absent_out, null_out],
            "P3 both open rows kept (absent and null clockOut), newest first; the closed \
             row of the same day dropped; got {:?}",
            ids(&got)
        );
    })
    .await;
}

// ── P4 ──────────────────────────────────────────────────────────────────────

/// P4: exactly S → S + 1 min is kept: `since` is inclusive.
#[tokio::test]
async fn p4_a_row_clocked_in_exactly_at_since_is_kept() {
    within("P4", async {
        let db = TestDb::start().await;
        let branch = db.branch("p4").await;
        let tag = unique("p4");
        let at_since = row(&format!("{tag}-at-since"), "e1", 0, Some(MINUTE));
        store(&db.pool, &branch, vec![at_since.clone()]).await;

        let got = since_all(&db, &branch).await;
        assert!(
            got == vec![at_since],
            "P4 clockIn == since is kept (inclusive); got {:?}",
            ids(&got)
        );
    })
    .await;
}

// ── P5 ──────────────────────────────────────────────────────────────────────

/// P5: S − 1 ms → S + 1 h is dropped: clocked in before S, even though it closed after.
#[tokio::test]
async fn p5_a_row_clocked_in_one_ms_before_since_and_closed_after_is_dropped() {
    within("P5", async {
        let db = TestDb::start().await;
        let branch = db.branch("p5").await;
        let tag = unique("p5");
        let dropped = row(&format!("{tag}-dropped"), "e1", -MS, Some(HOUR));
        let kept = row(&format!("{tag}-kept"), "e1", 0, Some(MINUTE));
        store(&db.pool, &branch, vec![dropped, kept.clone()]).await;

        let got = since_all(&db, &branch).await;
        assert!(
            got == vec![kept],
            "P5 clockIn = since − 1 ms is dropped; got {:?}",
            ids(&got)
        );
    })
    .await;
}

// ── P6 ──────────────────────────────────────────────────────────────────────

/// P6: several kept rows come newest first by `position` (not by time); `limit` counts
/// kept rows only; `before` answers the kept rows strictly older than the given row,
/// whether that row is kept or not; an id the branch does not hold answers nothing.
#[tokio::test]
async fn p6_kept_rows_come_newest_first_by_position_and_page_as_attendance_does() {
    within("P6", async {
        let db = TestDb::start().await;
        let branch = db.branch("p6").await;
        let tag = unique("p6");
        let n = |what: &str| format!("{tag}-{what}");
        // positions 0..=6; kept rows k*, dropped rows d*. k1's clockIn is later than k2's,
        // so time order and position order differ.
        let k0 = row(&n("k0"), "e1", MINUTE, Some(2 * MINUTE));
        let d0 = row(&n("d0"), "e1", -2 * HOUR, Some(-HOUR));
        let k1 = row(&n("k1"), "e2", 50 * MINUTE, None);
        let d1 = row(&n("d1"), "e2", -3 * HOUR, Some(-2 * HOUR));
        let k2 = row(&n("k2"), "e1", 10 * MINUTE, Some(20 * MINUTE));
        let d2 = row(&n("d2"), "e1", -MINUTE, Some(MINUTE));
        let k3 = row(&n("k3"), "e2", -DAY, None);
        store(
            &db.pool,
            &branch,
            vec![
                k0.clone(),
                d0,
                k1.clone(),
                d1.clone(),
                k2.clone(),
                d2.clone(),
                k3.clone(),
            ],
        )
        .await;
        let p = pages(&db);
        let page = |before: Option<String>, limit: u16| {
            let p = &p;
            let branch = &branch;
            async move {
                p.attendance_since(branch, None, since(), before.as_deref(), limit)
                    .await
                    .expect("attendance_since")
            }
        };

        let all = page(None, 50).await;
        assert!(
            all == vec![k3.clone(), k2.clone(), k1.clone(), k0.clone()],
            "P6 newest first by position; got {:?}",
            ids(&all)
        );

        let first = page(None, 2).await;
        assert!(
            first == vec![k3.clone(), k2.clone()],
            "P6 limit counts kept rows only; got {:?}",
            ids(&first)
        );

        let next = page(Some(k2.id.clone()), 2).await;
        assert!(
            next == vec![k1.clone(), k0.clone()],
            "P6 before a kept row; got {:?}",
            ids(&next)
        );

        let after_dropped = page(Some(d2.id.clone()), 50).await;
        assert!(
            after_dropped == vec![k2.clone(), k1.clone(), k0.clone()],
            "P6 before a dropped row pages by its position; got {:?}",
            ids(&after_dropped)
        );
        let after_d1 = page(Some(d1.id.clone()), 50).await;
        assert!(
            after_d1 == vec![k1, k0.clone()],
            "P6 before d1; got {:?}",
            ids(&after_d1)
        );

        assert!(
            page(Some(k0.id.clone()), 50).await.is_empty(),
            "P6 before the oldest kept row: nothing"
        );
        assert!(
            page(Some(unique("no-such-row")), 50).await.is_empty(),
            "P6 an id the branch does not hold: nothing"
        );
    })
    .await;
}

// ── P7 ──────────────────────────────────────────────────────────────────────

/// P7: `only = Some(e1)` answers e1's kept rows only.
#[tokio::test]
async fn p7_only_e1_answers_e1s_kept_rows_only() {
    within("P7", async {
        let db = TestDb::start().await;
        let branch = db.branch("p7").await;
        let tag = unique("p7");
        let n = |what: &str| format!("{tag}-{what}");
        let e1_kept_old = row(&n("e1-kept-old"), "e1", MINUTE, Some(2 * MINUTE));
        let e1_dropped = row(&n("e1-dropped"), "e1", -HOUR, Some(-MINUTE));
        let e2_kept = row(&n("e2-kept"), "e2", 3 * MINUTE, None);
        let e1_open = row(&n("e1-open"), "e1", -DAY, None);
        let e2_open = row(&n("e2-open"), "e2", -DAY, None);
        store(
            &db.pool,
            &branch,
            vec![
                e1_kept_old.clone(),
                e1_dropped,
                e2_kept.clone(),
                e1_open.clone(),
                e2_open.clone(),
            ],
        )
        .await;
        let p = pages(&db);

        let got = p
            .attendance_since(&branch, Some("e1"), since(), None, 50)
            .await
            .expect("attendance_since of e1");
        assert!(
            got == vec![e1_open.clone(), e1_kept_old.clone()],
            "P7 e1's kept rows, newest first; got {:?}",
            ids(&got)
        );

        let got = p
            .attendance_since(&branch, Some("e1"), since(), None, 1)
            .await
            .expect("first page of e1");
        assert!(
            got == vec![e1_open.clone()],
            "P7 limit applies to e1's kept rows"
        );
        let got = p
            .attendance_since(&branch, Some("e1"), since(), Some(&e1_open.id), 50)
            .await
            .expect("e1 before its newest");
        assert!(
            got == vec![e1_kept_old],
            "P7 before applies to e1's kept rows"
        );

        let got = p
            .attendance_since(&branch, Some("e2"), since(), None, 50)
            .await
            .expect("attendance_since of e2");
        assert!(got == vec![e2_open, e2_kept], "P7 e2's kept rows");

        assert!(
            p.attendance_since(&branch, Some("nobody"), since(), None, 50)
                .await
                .expect("attendance_since of nobody")
                .is_empty()
        );
    })
    .await;
}

// ── P8 ──────────────────────────────────────────────────────────────────────

/// P8: a kept row on another branch is not answered.
#[tokio::test]
async fn p8_a_kept_row_of_another_branch_is_not_answered() {
    within("P8", async {
        let db = TestDb::start().await;
        let mine = db.branch("p8mine").await;
        let other = db.branch("p8other").await;
        let tag = unique("p8");
        let my_kept = row(&format!("{tag}-mine"), "e1", HOUR, Some(2 * HOUR));
        store(&db.pool, &mine, vec![my_kept.clone()]).await;
        let their_kept = row(&format!("{tag}-other-kept"), "e1", 2 * HOUR, None);
        let their_closed = row(&format!("{tag}-other-closed"), "e1", HOUR, Some(3 * HOUR));
        store(&db.pool, &other, vec![their_kept.clone(), their_closed]).await;
        let p = pages(&db);

        let got = since_all(&db, &mine).await;
        assert!(
            got == vec![my_kept.clone()],
            "P8 only this branch's rows; got {:?}",
            ids(&got)
        );
        let got = p
            .attendance_since(&mine, Some("e1"), since(), None, 50)
            .await
            .expect("attendance_since of e1");
        assert!(
            got == vec![my_kept],
            "P8 e1 also works at the other branch; only this branch's rows"
        );
        assert!(
            p.attendance_since(&mine, None, since(), Some(&their_kept.id), 50)
                .await
                .expect("attendance_since before another branch's id")
                .is_empty(),
            "P8 an id of the other branch is not an id this branch holds"
        );
        assert_eq!(
            since_all(&db, &other).await.len(),
            2,
            "the other branch answers its own two kept rows"
        );
    })
    .await;
}
