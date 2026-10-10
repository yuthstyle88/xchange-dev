//! QU-A2 (#556 design row QU-4) — `CounterService` against a real PostgreSQL: a teller's counter
//! follows their OPEN shift under CA-3 (one OPEN shift per teller, so several tellers of one
//! branch each have a counter); pause and resume change `active_counters` by one; a PAUSED
//! counter cannot call; waiting tickets are untouched; the estimate follows. Written before the
//! implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` §3, §4.2 (`queue_counters`), §4.3, Q7, §9.1
//! row QU-4. Owner Q-A4 (pause is the teller's counter: it stops calling, `active_counters` drops
//! by one, the estimate updates, waiting tickets are untouched). CA-3: #619, PR 871.
//!
//! Signatures this test assumes (QU-A2 scaffolding, `[required]` once pushed):
//! - `CounterService::{pause, resume}(&Actor, &BranchId, now) -> AppResult<Counter>`;
//! - `CounterService::on_shift_opened(tx, &BranchId, teller_id, now) -> AppResult<Counter>` and
//!   `on_shift_closed(tx, &BranchId, teller_id, now) -> AppResult<()>`;
//! - `Counter { teller_id, status: CounterStatus, .. }`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, Utc};
use queue_common::*;
use queue_desk_common::*;
use xc_kernel::ids::BranchId;
use xc_queue::domain::counter::counter::CounterStatus;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;

async fn walk_in(db: &TestDb, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&main_branch(), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

async fn open_main() -> (TestDb, Queue) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let q = queue(&db);
    (db, q)
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

fn sorted(mut rows: Vec<String>) -> Vec<String> {
    rows.sort();
    rows
}

async fn active_rows(db: &TestDb, branch: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM queue_counters WHERE branch_id = '{branch}' AND status = 'ACTIVE'"
    ))
    .await
}

#[tokio::test]
async fn opening_a_shift_adds_an_active_counter_and_closing_it_removes_it() {
    let (db, q) = open_main().await;
    assert_eq!(active_counters(&db, "MAIN").await, 0);

    let a = db.add_user("Teller A").await;
    let b = db.add_user("Teller B").await;
    open_shift(&db, &q, "MAIN", &a, at(2, 0)).await;
    assert_eq!(counters(&db, "MAIN").await, vec![format!("{a} ACTIVE")]);
    assert_eq!(active_counters(&db, "MAIN").await, 1);

    open_shift(&db, &q, "MAIN", &b, at(2, 5)).await;
    assert_eq!(
        counters(&db, "MAIN").await,
        sorted(vec![format!("{a} ACTIVE"), format!("{b} ACTIVE")]),
        "CA-3: each teller of one branch holds an OPEN shift, so each has a counter"
    );
    assert_eq!(active_counters(&db, "MAIN").await, 2);

    close_shift(&db, &q, "MAIN", &a, at(9, 0)).await;
    assert_eq!(
        counters(&db, "MAIN").await,
        vec![format!("{b} ACTIVE")],
        "closing A's shift removes A's counter only"
    );
    assert_eq!(active_counters(&db, "MAIN").await, 1);
    close_shift(&db, &q, "MAIN", &b, at(9, 5)).await;
    assert_eq!(counters(&db, "MAIN").await, Vec::<String>::new());
    assert_eq!(active_counters(&db, "MAIN").await, 0);
    db.finish().await;
}

#[tokio::test]
async fn the_shift_hook_answers_the_new_active_counter_and_runs_in_the_shifts_transaction() {
    let (db, q) = open_main().await;
    let a = db.add_user("Teller A").await;
    let counters_svc = q.counters.clone();
    let teller_id = a.clone();
    let rolled_back = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                let counter = counters_svc
                    .on_shift_opened(tx, &BranchId::new("MAIN"), &teller_id, at(2, 0))
                    .await?;
                assert_eq!(
                    (
                        counter.teller_id.as_str(),
                        counter.status,
                        counter.branch_id.as_str()
                    ),
                    (teller_id.as_str(), CounterStatus::Active, "MAIN")
                );
                Err::<(), _>(pos_error::AppError::Validation(
                    "the shift's open fails after".to_owned(),
                ))
            })
        })
        .await;
    assert!(rolled_back.is_err());
    assert_eq!(
        counters(&db, "MAIN").await,
        Vec::<String>::new(),
        "the counter rolls back with the shift's open"
    );
    assert_eq!(active_counters(&db, "MAIN").await, 0);
    db.finish().await;
}

#[tokio::test]
async fn pause_drops_active_counters_by_one_and_resume_raises_it() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let br = main_branch();
    assert_eq!(active_counters(&db, "MAIN").await, 2);

    let paused = q
        .counters
        .pause(&teller(&a, "MAIN"), &br, at(3, 0))
        .await
        .expect("A pauses");
    assert_eq!(
        (paused.teller_id.as_str(), paused.status),
        (a.as_str(), CounterStatus::Paused)
    );
    assert_eq!(
        active_counters(&db, "MAIN").await,
        1,
        "pause drops active_counters by one (owner Q-A4)"
    );
    assert_eq!(
        counters(&db, "MAIN").await,
        sorted(vec![format!("{a} PAUSED"), format!("{b} ACTIVE")]),
        "only the caller's own counter"
    );

    q.counters
        .pause(&teller(&b, "MAIN"), &br, at(3, 1))
        .await
        .expect("B pauses");
    assert_eq!(active_counters(&db, "MAIN").await, 0);

    let resumed = q
        .counters
        .resume(&teller(&a, "MAIN"), &br, at(3, 2))
        .await
        .expect("A resumes");
    assert_eq!(resumed.status, CounterStatus::Active);
    assert_eq!(
        active_counters(&db, "MAIN").await,
        1,
        "resume raises it by one"
    );

    let _ = q.counters.pause(&teller(&b, "MAIN"), &br, at(3, 3)).await;
    let _ = q.counters.resume(&teller(&a, "MAIN"), &br, at(3, 3)).await;
    assert_eq!(
        active_counters(&db, "MAIN").await,
        active_rows(&db, "MAIN").await,
        "pausing a paused counter or resuming an active one never drifts the count from the rows"
    );
    assert_eq!(active_counters(&db, "MAIN").await, 1);
    db.finish().await;
}

#[tokio::test]
async fn a_paused_counter_cannot_call_and_waiting_tickets_are_untouched() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ta = teller(&a, "MAIN");
    let br = main_branch();
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let w2 = walk_in(&db, "w2", at(3, 1)).await;

    q.counters.pause(&ta, &br, at(3, 5)).await.expect("pause");
    assert_eq!(
        refused(q.desk.call_next(&ta, &br, at(3, 6)).await),
        "counterPaused"
    );
    assert_eq!(
        refused(q.desk.call(&ta, &br, &w2.id, at(3, 6)).await),
        "counterPaused"
    );
    for (w, n) in [(&w1, "W001"), (&w2, "W002")] {
        assert_eq!(
            stored(&db, &w.id).await,
            format!("WAITING {n}"),
            "waiting tickets are untouched (owner Q-A4)"
        );
        assert_eq!(events(&db, &w.id).await, vec!["ISSUE WAITING -".to_owned()]);
    }

    q.counters.resume(&ta, &br, at(3, 7)).await.expect("resume");
    let called = q
        .desk
        .call_next(&ta, &br, at(3, 8))
        .await
        .expect("an ACTIVE counter calls again");
    assert_eq!(called.id, w1.id);
    db.finish().await;
}

#[tokio::test]
async fn only_a_teller_on_duty_in_the_branch_pauses_and_only_their_own_counter() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let off = db.add_user("Teller Off").await;
    let cust = db.add_user("A customer").await;
    let br = main_branch();

    assert_eq!(
        refused(q.counters.pause(&teller(&off, "MAIN"), &br, at(3, 0)).await),
        "noOpenShift",
        "no OPEN shift, no counter (Q7)"
    );
    assert_eq!(
        refused(
            q.counters
                .resume(&teller(&off, "MAIN"), &br, at(3, 0))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(
        refused(q.counters.pause(&customer(&cust), &br, at(3, 0)).await),
        "forbidden"
    );
    assert_eq!(
        refused(q.counters.pause(&teller(&a, "B2"), &br, at(3, 0)).await),
        "forbidden",
        "QueueOperate in another branch is not QueueOperate in MAIN"
    );
    assert_eq!(
        counters(&db, "MAIN").await,
        vec![format!("{a} ACTIVE")],
        "nobody paused A's counter"
    );
    assert_eq!(active_counters(&db, "MAIN").await, 1);
    db.finish().await;
}

#[tokio::test]
async fn pausing_and_resuming_update_the_estimate() {
    let (db, q) = open_main().await;
    // Today's sample: 60, 120, 180 s → 25th/50th/75th percentile 90 / 120 / 150 (owner Q-A5, Q-A6).
    for (seq, secs) in [(500, 60), (501, 120), (502, 180)] {
        add_completed(&db, "MAIN", day(2), seq, secs, at(2, 30)).await;
    }
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 40)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 40)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    walk_in(&db, "w2", at(3, 1)).await;
    refresh(&db, &q, "MAIN", at(3, 2)).await;
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("120 90 150".to_owned(), true),
        "2 ahead ÷ 2 counters × (120, 90, 150): estimate, low, high"
    );

    let br = main_branch();
    q.counters
        .pause(&teller(&a, "MAIN"), &br, at(3, 3))
        .await
        .expect("A pauses");
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("240 180 300".to_owned(), true),
        "the estimate updates in the pause's own transaction (owner Q-A4)"
    );
    q.counters
        .pause(&teller(&b, "MAIN"), &br, at(3, 4))
        .await
        .expect("B pauses");
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("NULL NULL NULL".to_owned(), true),
        "no active counter: unknown, with as_of"
    );
    q.counters
        .resume(&teller(&b, "MAIN"), &br, at(3, 5))
        .await
        .expect("B resumes");
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("240 180 300".to_owned(), true)
    );
    db.finish().await;
}
