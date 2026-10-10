//! QU-A2 (#556 design row QU-5, the day close) — `QueueDayHook::on_day_closed` against a real
//! PostgreSQL, called directly: every WAITING or CALLED ticket of the branch becomes CANCELLED with
//! `ended_reason = 'dayClosed'`, with one CANCEL event each; nothing else moves; it runs in the
//! caller's transaction and refreshes the status. The call from `BranchDay` is not here (the lane
//! controller's ruling 3 on #880). Written before the implementation by the ticket author
//! (claude/b060e9). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` §4.2, §4.3 (last row), Q8, §9.1 row QU-5.
//!
//! Signature this test assumes (QU-A2 scaffolding, `[required]` once pushed):
//! `QueueDayHook::on_day_closed(tx, &BranchId, now) -> AppResult<Vec<QueueTicket>>`, the tickets
//! it cancelled.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, Utc};
use pos_error::AppError;
use queue_common::*;
use queue_desk_common::*;
use xc_kernel::ids::BranchId;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};

async fn issue(
    db: &TestDb,
    branch: &str,
    source: IssueSource,
    key: &str,
    now: DateTime<Utc>,
) -> QueueTicket {
    db.issuer()
        .issue(&BranchId::new(branch), source, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

async fn close_day(db: &TestDb, q: &Queue, branch: &str, now: DateTime<Utc>) -> Vec<QueueTicket> {
    let day = q.day.clone();
    let b = BranchId::new(branch);
    db.branch_tx()
        .run(&BranchId::new(branch), move |tx| {
            Box::pin(async move { day.on_day_closed(tx, &b, now).await })
        })
        .await
        .unwrap_or_else(|e| panic!("on_day_closed on {branch}: {e:?}"))
}

/// `status reason` of a ticket as stored (`-` for no reason).
async fn ended(db: &TestDb, id: &str) -> String {
    db.text(&format!(
        "SELECT status || ' ' || COALESCE(ended_reason, '-') FROM queue_tickets WHERE id = '{id}'::uuid"
    ))
    .await
    .expect("stored")
}

/// MAIN with one ticket in each state; answers them by state name.
struct Board {
    completed: QueueTicket,
    serving: QueueTicket,
    no_show: QueueTicket,
    called: QueueTicket,
    cancelled: QueueTicket,
    waiting: QueueTicket,
    booked_waiting: QueueTicket,
}

async fn board(db: &TestDb, q: &Queue) -> Board {
    db.open_queue("MAIN").await;
    let a = on_duty(db, q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(db, q, "MAIN", "Teller B", at(2, 0)).await;
    let c = on_duty(db, q, "MAIN", "Teller C", at(2, 0)).await;
    let (ta, tb, tc, br) = (
        teller(&a, "MAIN"),
        teller(&b, "MAIN"),
        teller(&c, "MAIN"),
        main_branch(),
    );
    let mut w = Vec::new();
    for k in 1..=6u32 {
        w.push(issue(db, "MAIN", IssueSource::Desk, &format!("w{k}"), at(3, k)).await);
    }
    let desk = &q.desk;
    desk.call(&ta, &br, &w[0].id, at(4, 0)).await.expect("call");
    desk.start(&ta, &br, &w[0].id, at(4, 1))
        .await
        .expect("start");
    let completed = desk
        .finish(&ta, &br, &w[0].id, at(4, 2))
        .await
        .expect("finish");
    desk.call(&tb, &br, &w[1].id, at(4, 3)).await.expect("call");
    let serving = desk
        .start(&tb, &br, &w[1].id, at(4, 4))
        .await
        .expect("start");
    desk.call(&tc, &br, &w[2].id, at(4, 5)).await.expect("call");
    let no_show = desk.skip(&tc, &br, &w[2].id, at(4, 6)).await.expect("skip");
    let called = desk.call(&tc, &br, &w[3].id, at(4, 7)).await.expect("call");
    let cancelled = desk
        .cancel(&ta, &br, &w[4].id, "customerLeft", at(4, 8))
        .await
        .expect("cancel");
    db.add_booking("MAIN", "BK-1").await;
    let booked_waiting = issue(
        db,
        "MAIN",
        IssueSource::Booking {
            booking_id: "BK-1".to_owned(),
        },
        "booking:BK-1",
        at(4, 9),
    )
    .await;
    Board {
        completed,
        serving,
        no_show,
        called,
        cancelled,
        waiting: w[5].clone(),
        booked_waiting,
    }
}

#[tokio::test]
async fn the_day_close_cancels_waiting_and_called_tickets_with_day_closed() {
    let db = TestDb::start().await;
    let q = queue(&db);
    let bd = board(&db, &q).await;

    let mut cancelled: Vec<String> = close_day(&db, &q, "MAIN", at(12, 0))
        .await
        .into_iter()
        .map(|t| {
            assert_eq!(
                (t.status, t.ended_reason.as_deref()),
                (TicketStatus::Cancelled, Some("dayClosed")),
                "{} as answered",
                t.display_number
            );
            t.id
        })
        .collect();
    cancelled.sort();
    let mut expected = vec![
        bd.called.id.clone(),
        bd.waiting.id.clone(),
        bd.booked_waiting.id.clone(),
    ];
    expected.sort();
    assert_eq!(
        cancelled, expected,
        "exactly the WAITING and CALLED tickets, both lines"
    );

    for t in [&bd.called, &bd.waiting, &bd.booked_waiting] {
        assert_eq!(
            ended(&db, &t.id).await,
            "CANCELLED dayClosed",
            "{}",
            t.display_number
        );
        let last = events(&db, &t.id).await.pop().expect("events");
        assert!(
            last.starts_with("CANCEL CANCELLED "),
            "{}: {last}",
            t.display_number
        );
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM queue_events WHERE ticket_id = '{}'::uuid AND kind = 'CANCEL' \
                 AND reason = 'dayClosed'",
                t.id
            ))
            .await,
            1,
            "{}: one CANCEL event with the reason",
            t.display_number
        );
    }
    assert_eq!(
        ended(&db, &bd.serving.id).await,
        "SERVING -",
        "SERVING ends only in COMPLETED (Q5)"
    );
    assert_eq!(ended(&db, &bd.completed.id).await, "COMPLETED -");
    assert_eq!(ended(&db, &bd.no_show.id).await, "NO_SHOW -");
    assert_eq!(
        ended(&db, &bd.cancelled.id).await,
        "CANCELLED customerLeft",
        "an earlier cancel keeps its reason"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM queue_events WHERE reason = 'dayClosed'")
            .await,
        3,
        "no event for the tickets that did not move"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_day_close_touches_only_its_branch_and_refreshes_the_status() {
    let db = TestDb::start().await;
    let q = queue(&db);
    db.open_queue("MAIN").await;
    db.add_branch("B2", "Asia/Bangkok").await;
    db.open_queue("B2").await;
    let mine = issue(&db, "MAIN", IssueSource::Desk, "m1", at(3, 0)).await;
    let theirs = issue(&db, "B2", IssueSource::Desk, "b1", at(3, 1)).await;

    let cancelled = close_day(&db, &q, "MAIN", at(12, 0)).await;
    assert_eq!(cancelled.len(), 1);
    assert_eq!(ended(&db, &mine.id).await, "CANCELLED dayClosed");
    assert_eq!(
        ended(&db, &theirs.id).await,
        "WAITING -",
        "another branch's day is not closed"
    );

    let as_of: (DateTime<Utc>,) =
        sqlx::query_as("SELECT estimate_as_of FROM branch_service_status WHERE branch_id = 'MAIN'")
            .fetch_one(&db.pool)
            .await
            .expect("MAIN has a status row");
    assert_eq!(
        as_of.0,
        at(12, 0),
        "StatusService::refresh_in ran in the day close"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_day_close_runs_in_the_callers_transaction_and_rolls_back_with_it() {
    let db = TestDb::start().await;
    let q = queue(&db);
    db.open_queue("MAIN").await;
    let w1 = issue(&db, "MAIN", IssueSource::Desk, "w1", at(3, 0)).await;

    let day = q.day.clone();
    let failed = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                let cancelled = day
                    .on_day_closed(tx, &BranchId::new("MAIN"), at(12, 0))
                    .await?;
                assert_eq!(cancelled.len(), 1);
                Err::<(), _>(AppError::Validation(
                    "the branch day's close fails after".to_owned(),
                ))
            })
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(
        ended(&db, &w1.id).await,
        "WAITING -",
        "the cancel rolls back with the caller"
    );
    assert_eq!(
        events(&db, &w1.id).await,
        vec!["ISSUE WAITING -".to_owned()]
    );

    assert_eq!(close_day(&db, &q, "MAIN", at(12, 1)).await.len(), 1);
    assert!(
        close_day(&db, &q, "MAIN", at(12, 2)).await.is_empty(),
        "a second close finds nothing left to cancel"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM queue_events WHERE reason = 'dayClosed'")
            .await,
        1
    );
    db.finish().await;
}
