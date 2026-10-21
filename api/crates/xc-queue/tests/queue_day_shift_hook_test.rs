//! #4136 Q1: `QueueDayHook` as the shift's `ShiftLifecycleHook`. Design
//! `docs/design/blueprint/queue-day-close.md` §2 row 1 and §3.4: `on_day_closed`, called through
//! `Arc<dyn ShiftLifecycleHook>` on the caller's transaction, does what `QueueDayHook::on_day_closed` does —
//! WAITING and CALLED become CANCELLED with `dayClosed` and one CANCEL event each, nothing else moves — and the
//! other two methods do nothing.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-9). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use queue_common::*;
use queue_desk_common::*;
use xc_kernel::ids::BranchId;
use xc_kernel::shift_hook::ShiftLifecycleHook;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;

async fn issue(db: &TestDb, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&BranchId::new("MAIN"), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

/// `status reason` of a ticket as stored (`-` for no reason).
async fn ended(db: &TestDb, id: &str) -> String {
    db.text(&format!(
        "SELECT status || ' ' || COALESCE(ended_reason, '-') FROM queue_tickets WHERE id = '{id}'::uuid"
    ))
    .await
    .expect("stored")
}

async fn cancel_events(db: &TestDb, id: &str) -> String {
    db.text(&format!(
        "SELECT count(*)::text FROM queue_events WHERE ticket_id = '{id}'::uuid AND kind = 'CANCEL'"
    ))
    .await
    .expect("events")
}

/// MAIN with a COMPLETED, a SERVING, a CALLED and a WAITING ticket.
struct Board {
    completed: QueueTicket,
    serving: QueueTicket,
    called: QueueTicket,
    waiting: QueueTicket,
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
    for k in 1..=4u32 {
        w.push(issue(db, &format!("q1-{k}"), at(3, k)).await);
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
    let called = desk.call(&tc, &br, &w[2].id, at(4, 5)).await.expect("call");
    Board {
        completed,
        serving,
        called,
        waiting: w[3].clone(),
    }
}

fn as_hook(q: &Queue) -> Arc<dyn ShiftLifecycleHook> {
    Arc::new(q.day.clone())
}

/// Row 1: through the trait object, WAITING and CALLED become `CANCELLED dayClosed` with one CANCEL event each;
/// SERVING and COMPLETED are unchanged.
#[tokio::test]
async fn row1_on_day_closed_through_the_trait_cancels_waiting_and_called() {
    let db = TestDb::start().await;
    let q = queue(&db);
    let bd = board(&db, &q).await;
    let hook = as_hook(&q);

    let b = BranchId::new("MAIN");
    db.branch_tx()
        .run(&BranchId::new("MAIN"), move |tx| {
            Box::pin(async move { hook.on_day_closed(tx, &b, at(12, 0)).await })
        })
        .await
        .expect("on_day_closed through ShiftLifecycleHook");

    assert_eq!(ended(&db, &bd.waiting.id).await, "CANCELLED dayClosed");
    assert_eq!(ended(&db, &bd.called.id).await, "CANCELLED dayClosed");
    assert_eq!(ended(&db, &bd.serving.id).await, "SERVING -");
    assert_eq!(ended(&db, &bd.completed.id).await, "COMPLETED -");
    assert_eq!(cancel_events(&db, &bd.waiting.id).await, "1");
    assert_eq!(cancel_events(&db, &bd.called.id).await, "1");
    assert_eq!(cancel_events(&db, &bd.serving.id).await, "0");
}

/// Row 1: the hook works on the caller's transaction — a caller that rolls back leaves every ticket as it was.
#[tokio::test]
async fn row1_a_rolled_back_caller_leaves_the_tickets() {
    let db = TestDb::start().await;
    let q = queue(&db);
    let bd = board(&db, &q).await;
    let hook = as_hook(&q);

    let b = BranchId::new("MAIN");
    let answer: pos_error::AppResult<()> = db
        .branch_tx()
        .run(&BranchId::new("MAIN"), move |tx| {
            Box::pin(async move {
                hook.on_day_closed(tx, &b, at(12, 0)).await?;
                Err(pos_error::AppError::Domain(
                    "the caller fails after the hook".into(),
                ))
            })
        })
        .await;
    assert!(answer.is_err());

    assert_eq!(ended(&db, &bd.waiting.id).await, "WAITING -");
    assert_eq!(ended(&db, &bd.called.id).await, "CALLED -");
    assert_eq!(cancel_events(&db, &bd.waiting.id).await, "0");
}

/// §3.4: a shift opening or closing is `CounterService`'s; `QueueDayHook` answers both with nothing.
#[tokio::test]
async fn opened_and_closed_do_nothing() {
    let db = TestDb::start().await;
    let q = queue(&db);
    let bd = board(&db, &q).await;
    let hook = as_hook(&q);

    let b = BranchId::new("MAIN");
    db.branch_tx()
        .run(&BranchId::new("MAIN"), move |tx| {
            Box::pin(async move {
                hook.on_opened(tx, &b, "00000000-0000-0000-0000-0000000000aa", at(12, 0))
                    .await?;
                hook.on_closed(tx, &b, "00000000-0000-0000-0000-0000000000aa", at(12, 1))
                    .await
            })
        })
        .await
        .expect("on_opened and on_closed");

    assert_eq!(ended(&db, &bd.waiting.id).await, "WAITING -");
    assert_eq!(ended(&db, &bd.called.id).await, "CALLED -");
}
