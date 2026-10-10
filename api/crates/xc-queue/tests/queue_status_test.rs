//! QU-A2 (#556 design row QU-5) — `StatusService` against a real PostgreSQL: the wait estimate
//! and its range from the branch's service times (owner Q-A5, Q-A6), unknown when there is
//! nothing to compute from, `estimate_as_of`; opening and closing the branch queue; issuance
//! refreshing the status. Written before the implementation by the ticket author
//! (claude/b060e9). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` §3, §4.3, Q8, Q9, Q13, §9.1 row QU-5.
//! - Q-A5: today's median once at least 3 tickets are completed today; before that, the median of
//!   the last 30 completed tickets of earlier days.
//! - Q-A6: the range is the 25th–75th percentile of the same sample (`percentile_cont`).
//! - `estimate` = tickets ahead (the WAITING tickets) ÷ `active_counters` × the percentile.
//!
//! Every sample here gives whole seconds, so no rounding rule is assumed.
//!
//! Signatures this test assumes (QU-A2 scaffolding, `[required]` once pushed):
//! - `StatusService::{open, close}(&Actor, &BranchId, now) -> AppResult<BranchServiceStatus>`;
//! - `StatusService::refresh_in(tx, &BranchId, now) -> AppResult<BranchServiceStatus>`;
//! - `BranchServiceStatus { queue_open, active_counters, estimate: Option<WaitEstimate>,
//!   estimate_as_of, .. }`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_error::AppError;
use queue_common::*;
use queue_desk_common::*;
use xc_kernel::ids::BranchId;
use xc_queue::domain::status::wait_estimator::WaitEstimate;
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

/// 30 tickets completed on 1 November, served 40, 80, …, 1200 s (the newest last), and one older
/// ticket on 31 October served 10 000 s that the last-30 window must leave out.
/// Percentiles of the 30: 25th = 330, 50th = 620, 75th = 910.
async fn thirty_earlier(db: &TestDb, branch: &str) {
    let oct31 = Utc.with_ymd_and_hms(2026, 10, 31, 5, 0, 0).unwrap();
    add_completed(
        db,
        branch,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
        500,
        10_000,
        oct31,
    )
    .await;
    let nov1 = Utc.with_ymd_and_hms(2026, 11, 1, 3, 0, 0).unwrap();
    for k in 1..=30 {
        add_completed(
            db,
            branch,
            day(1),
            500 + k as i32,
            40 * k,
            nov1 + Duration::minutes(k),
        )
        .await;
    }
}

#[tokio::test]
async fn with_two_completed_today_the_estimate_is_the_median_of_the_last_30_of_earlier_days() {
    let (db, q) = open_main().await;
    thirty_earlier(&db, "MAIN").await;
    add_completed(&db, "MAIN", day(2), 600, 10, at(2, 0)).await;
    add_completed(&db, "MAIN", day(2), 601, 20, at(2, 5)).await;
    on_duty(&db, &q, "MAIN", "Teller A", at(2, 10)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    refresh(&db, &q, "MAIN", at(3, 1)).await;
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("620 330 910".to_owned(), true),
        "1 ahead on 1 counter × the earlier sample (owner Q-A5): not today's 2 tickets, and not the \
         31st-newest earlier ticket"
    );
    db.finish().await;
}

#[tokio::test]
async fn with_three_completed_today_the_estimate_is_todays_median() {
    let (db, q) = open_main().await;
    thirty_earlier(&db, "MAIN").await;
    for (seq, secs) in [(600, 60), (601, 120), (602, 180)] {
        add_completed(&db, "MAIN", day(2), seq, secs, at(2, 0)).await;
    }
    on_duty(&db, &q, "MAIN", "Teller A", at(2, 10)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    refresh(&db, &q, "MAIN", at(3, 1)).await;
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("120 90 150".to_owned(), true),
        "today has 3: today's sample only (owner Q-A5)"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_range_is_the_25th_to_75th_percentile_of_the_same_sample() {
    let (db, q) = open_main().await;
    for (seq, secs) in [(600, 240), (601, 60), (602, 180), (603, 120)] {
        add_completed(&db, "MAIN", day(2), seq, secs, at(2, 0)).await;
    }
    on_duty(&db, &q, "MAIN", "Teller A", at(2, 10)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    walk_in(&db, "w2", at(3, 1)).await;

    let status = q.status.clone();
    let st = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                status
                    .refresh_in(tx, &BranchId::new("MAIN"), at(3, 2))
                    .await
            })
        })
        .await
        .expect("refresh");
    assert_eq!(
        st.estimate,
        Some(WaitEstimate {
            seconds: 300,
            low_seconds: 210,
            high_seconds: 390,
        }),
        "2 ahead on 1 counter × percentile_cont 0.5 / 0.25 / 0.75 of 60, 120, 180, 240 = 150 / 105 / 195"
    );
    assert_eq!((st.active_counters, st.queue_open), (1, true));
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("300 210 390".to_owned(), true),
        "and stored"
    );
    let e = st.estimate.unwrap();
    assert!(e.low_seconds <= e.seconds && e.seconds <= e.high_seconds);
    db.finish().await;
}

#[tokio::test]
async fn the_tickets_ahead_are_shared_by_the_active_counters() {
    let (db, q) = open_main().await;
    for (seq, secs) in [(600, 60), (601, 120), (602, 180), (603, 240)] {
        add_completed(&db, "MAIN", day(2), seq, secs, at(2, 0)).await;
    }
    on_duty(&db, &q, "MAIN", "Teller A", at(2, 10)).await;
    on_duty(&db, &q, "MAIN", "Teller B", at(2, 10)).await;
    for k in 1..=4 {
        walk_in(&db, &format!("w{k}"), at(3, k)).await;
    }
    refresh(&db, &q, "MAIN", at(3, 10)).await;
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("300 210 390".to_owned(), true),
        "4 waiting ÷ 2 counters × (150, 105, 195)"
    );
    db.finish().await;
}

#[tokio::test]
async fn with_no_completed_ticket_today_or_before_the_estimate_is_unknown() {
    let (db, q) = open_main().await;
    db.add_branch("B2", "Asia/Bangkok").await;
    for (seq, secs) in [(600, 60), (601, 120), (602, 180)] {
        add_completed(&db, "B2", day(2), seq, secs, at(2, 0)).await;
    }
    on_duty(&db, &q, "MAIN", "Teller A", at(2, 10)).await;
    walk_in(&db, "w1", at(3, 0)).await;
    refresh(&db, &q, "MAIN", at(3, 1)).await;
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("NULL NULL NULL".to_owned(), true),
        "no sample of MAIN's own (another branch's service times do not count): unknown, never a \
         hard-coded number (Q9), and as_of is set"
    );
    db.finish().await;
}

#[tokio::test]
async fn with_no_active_counter_the_estimate_is_unknown() {
    let (db, q) = open_main().await;
    for (seq, secs) in [(600, 60), (601, 120), (602, 180)] {
        add_completed(&db, "MAIN", day(2), seq, secs, at(2, 0)).await;
    }
    walk_in(&db, "w1", at(3, 0)).await;
    walk_in(&db, "w2", at(3, 1)).await;
    refresh(&db, &q, "MAIN", at(3, 2)).await;
    assert_eq!(active_counters(&db, "MAIN").await, 0);
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("NULL NULL NULL".to_owned(), true)
    );
    db.finish().await;
}

#[tokio::test]
async fn estimate_as_of_is_when_the_estimate_was_computed() {
    let (db, q) = open_main().await;
    refresh(&db, &q, "MAIN", at(3, 7)).await;
    let as_of: (DateTime<Utc>,) =
        sqlx::query_as("SELECT estimate_as_of FROM branch_service_status WHERE branch_id = 'MAIN'")
            .fetch_one(&db.pool)
            .await
            .expect("as_of is set");
    assert_eq!(as_of.0, at(3, 7));
    db.finish().await;
}

#[tokio::test]
async fn issuing_a_ticket_refreshes_the_status_in_its_transaction() {
    let (db, q) = open_main().await;
    for (seq, secs) in [(600, 60), (601, 120), (602, 180)] {
        add_completed(&db, "MAIN", day(2), seq, secs, at(2, 0)).await;
    }
    on_duty(&db, &q, "MAIN", "Teller A", at(2, 10)).await;
    let before = queue_version(&db, "MAIN").await;

    walk_in(&db, "w1", at(3, 0)).await;
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("120 90 150".to_owned(), true),
        "TicketIssuer runs StatusService::refresh_in after an issue: 1 ahead"
    );
    let after_one = queue_version(&db, "MAIN").await;
    assert!(
        after_one > before,
        "the issue raises queue_version ({before} → {after_one})"
    );

    walk_in(&db, "w2", at(3, 1)).await;
    assert_eq!(
        estimate(&db, "MAIN").await,
        ("240 180 300".to_owned(), true),
        "2 ahead"
    );
    assert!(queue_version(&db, "MAIN").await > after_one);
    db.finish().await;
}

fn is_queue_closed(err: &AppError) -> bool {
    matches!(err, AppError::Validation(code) if code == "queueClosed")
}

#[tokio::test]
async fn closing_the_queue_stops_issuance_and_keeps_serving() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let boss = db.add_user("Manager").await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let br = main_branch();

    let closed = q
        .status
        .close(&manager(&boss, "MAIN"), &br, at(3, 5))
        .await
        .expect("close");
    assert!(!closed.queue_open);
    assert_eq!(
        db.text("SELECT queue_open::text FROM branch_service_status WHERE branch_id = 'MAIN'")
            .await
            .as_deref(),
        Some("false")
    );
    let refused_issue = db
        .issuer()
        .issue(&br, IssueSource::Desk, "w2", at(3, 6))
        .await
        .expect_err("a closed queue issues nothing");
    assert!(is_queue_closed(&refused_issue), "{refused_issue:?}");

    let ta = teller(&a, "MAIN");
    let called = q
        .desk
        .call_next(&ta, &br, at(3, 7))
        .await
        .expect("existing tickets are still called (Q8)");
    assert_eq!(called.id, w1.id);
    q.desk
        .start(&ta, &br, &w1.id, at(3, 8))
        .await
        .expect("and served");
    q.desk
        .finish(&ta, &br, &w1.id, at(3, 9))
        .await
        .expect("and finished");
    assert_eq!(stored(&db, &w1.id).await, "COMPLETED W001");

    let reopened = q
        .status
        .open(&manager(&boss, "MAIN"), &br, at(3, 10))
        .await
        .expect("reopen");
    assert!(reopened.queue_open);
    let w2 = walk_in(&db, "w2", at(3, 11)).await;
    assert_eq!(w2.display_number, "W002");
    db.finish().await;
}

#[tokio::test]
async fn only_branch_day_operate_opens_or_closes_and_open_creates_a_new_branchs_row() {
    let db = TestDb::start().await;
    let q = queue(&db);
    db.add_branch("B2", "Asia/Bangkok").await;
    let b2 = BranchId::new("B2");
    let tel = db.add_user("Teller").await;
    let boss = db.add_user("Manager").await;
    let cust = db.add_user("A customer").await;

    for (who, actor) in [
        ("a TELLER", teller(&tel, "B2")),
        ("a BRANCH_MANAGER of another branch", manager(&boss, "MAIN")),
        ("a customer", customer(&cust)),
    ] {
        assert_eq!(
            refused(q.status.open(&actor, &b2, at(3, 0)).await),
            "forbidden",
            "{who} cannot open"
        );
        assert_eq!(
            refused(q.status.close(&actor, &b2, at(3, 0)).await),
            "forbidden",
            "{who} cannot close"
        );
    }
    assert_eq!(
        db.count("SELECT count(*) FROM branch_service_status WHERE branch_id = 'B2'")
            .await,
        0,
        "the refusals created no row"
    );

    let opened = q
        .status
        .open(&manager(&boss, "B2"), &b2, at(3, 1))
        .await
        .expect("BRANCH_MANAGER opens");
    assert_eq!(
        (
            opened.branch_id.as_str(),
            opened.organization_id.as_str(),
            opened.queue_open
        ),
        ("B2", ORG_1, true),
        "a branch created after M1 gets its row on open (the lane controller's ruling 5 on #879)"
    );
    let first = db
        .issuer()
        .issue(&b2, IssueSource::Desk, "b2-1", at(3, 2))
        .await
        .expect("an open queue issues");
    assert_eq!(first.display_number, "W001");
    db.finish().await;
}
