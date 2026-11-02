//! #5675 (design `docs/design/blueprint/queue-tickets.md` §4.7, owner ruling A 2026-10-30 13:5x ICT): a shift close
//! never leaves an orphaned CALLED or SERVING ticket. `CounterService::on_shift_closed` refuses with
//! `ticketStillServing` while the closing teller holds a CALLED or SERVING ticket in the branch, and the refusal
//! rolls the close back: the counter stays and the ticket keeps its status. Nothing is released or cancelled
//! automatically; the teller finishes, releases (#5541) or skips it first. The routes that close a shift (the CLOSE
//! verify, a hand-over accept) are pinned in `api/tests/shift_close_live_ticket_route_test.rs`.
//!
//! Written before the implementation by the sub-controller (claude/01TxaRJG). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! Signature this test assumes (unchanged, [required]):
//! `CounterService::on_shift_closed(tx, &BranchId, teller_id, now) -> AppResult<()>`, and
//! `QueueError::TicketStillServing` with the code `"ticketStillServing"` (skeleton on `test/5675`).
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
use xc_queue::domain::ticket::queue_error::QueueError;
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

/// The shift close as the CLOSE verify runs it: the shift row and `on_shift_closed` in one branch transaction,
/// so a refusal rolls both back. Answers the hook's result.
async fn try_close(
    db: &TestDb,
    q: &Queue,
    teller_id: &str,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    let counters = q.counters.clone();
    let t = teller_id.to_owned();
    db.branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                sqlx::query(
                    "UPDATE shifts SET status = 'CLOSED', closed_at = now(), closed_by = $1 \
                     WHERE branch_id = 'MAIN' AND teller_id = $1::uuid AND status = 'OPEN'",
                )
                .bind(&t)
                .execute(&mut **tx)
                .await?;
                counters
                    .on_shift_closed(tx, &BranchId::new("MAIN"), &t, now)
                    .await
            })
        })
        .await
}

/// `teller status` of the teller's counter in MAIN, or `-` when it has none.
async fn counter_of(db: &TestDb, teller_id: &str) -> String {
    db.text(&format!(
        "SELECT COALESCE((SELECT status FROM queue_counters WHERE branch_id = 'MAIN' \
         AND teller_id = '{teller_id}'::uuid), '-')"
    ))
    .await
    .unwrap_or_else(|| "-".to_owned())
}

async fn shift_status(db: &TestDb, teller_id: &str) -> String {
    db.text(&format!(
        "SELECT status FROM shifts WHERE branch_id = 'MAIN' AND teller_id = '{teller_id}'::uuid \
         ORDER BY opened_at DESC LIMIT 1"
    ))
    .await
    .expect("the teller has a shift")
}

// Restated 2026-11-02 by claude/b091f5e0 (queue-tickets.md §4.7 "The refusal names the blocking tickets"): the
// refusal's message now carries the tickets after a marker, so it starts with the code instead of equalling it.
fn still_serving() -> String {
    QueueError::TicketStillServing(Vec::new()).code().to_owned()
}

/// R-1: the teller's CALLED ticket refuses the close; the shift stays OPEN, the counter stays, the ticket stays CALLED.
#[tokio::test]
async fn a_called_ticket_refuses_the_close_and_nothing_moves() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let t = walk_in(&db, "r1", at(2, 1)).await;
    q.desk
        .call(&teller(&a, "MAIN"), &main_branch(), &t.id, at(2, 2))
        .await
        .expect("call");
    let before = branch_version(&db, "MAIN").await;

    assert!(
        refused(try_close(&db, &q, &a, at(3, 0)).await).starts_with(&still_serving()),
        "the refusal is ticketStillServing"
    );

    assert_eq!(shift_status(&db, &a).await, "OPEN", "the close rolls back");
    assert_eq!(counter_of(&db, &a).await, "ACTIVE", "the counter stays");
    assert_eq!(
        stored(&db, &t.id).await,
        format!("CALLED {}", t.display_number)
    );
    assert_eq!(active_counters(&db, "MAIN").await, 1);
    assert_eq!(
        branch_version(&db, "MAIN").await,
        before,
        "a refused close commits nothing"
    );
    db.finish().await;
}

/// R-2: the teller's SERVING ticket refuses the close the same way.
#[tokio::test]
async fn a_serving_ticket_refuses_the_close_and_nothing_moves() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let t = walk_in(&db, "r2", at(2, 1)).await;
    let me = teller(&a, "MAIN");
    q.desk
        .call(&me, &main_branch(), &t.id, at(2, 2))
        .await
        .expect("call");
    q.desk
        .start(&me, &main_branch(), &t.id, at(2, 3))
        .await
        .expect("start");

    assert!(
        refused(try_close(&db, &q, &a, at(3, 0)).await).starts_with(&still_serving()),
        "the refusal is ticketStillServing"
    );

    assert_eq!(shift_status(&db, &a).await, "OPEN");
    assert_eq!(counter_of(&db, &a).await, "ACTIVE");
    assert_eq!(
        stored(&db, &t.id).await,
        format!("SERVING {}", t.display_number)
    );
    db.finish().await;
}

/// R-3: a PAUSED counter with a SERVING ticket is refused too; the pause does not hide the ticket.
#[tokio::test]
async fn a_paused_counter_with_a_serving_ticket_is_refused() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let t = walk_in(&db, "r3", at(2, 1)).await;
    let me = teller(&a, "MAIN");
    q.desk
        .call(&me, &main_branch(), &t.id, at(2, 2))
        .await
        .expect("call");
    q.desk
        .start(&me, &main_branch(), &t.id, at(2, 3))
        .await
        .expect("start");
    q.counters
        .pause(&me, &main_branch(), at(2, 4))
        .await
        .expect("pause");

    assert!(
        refused(try_close(&db, &q, &a, at(3, 0)).await).starts_with(&still_serving()),
        "the refusal is ticketStillServing"
    );
    assert_eq!(counter_of(&db, &a).await, "PAUSED");
    db.finish().await;
}

/// R-4: once the teller ends the ticket (finish, release, or skip), the same close goes through and removes the counter.
#[tokio::test]
async fn after_finish_release_or_skip_the_close_goes_through() {
    for end in ["finish", "release", "skip"] {
        let (db, q) = open_main().await;
        let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
        let t = walk_in(&db, &format!("r4-{end}"), at(2, 1)).await;
        let me = teller(&a, "MAIN");
        q.desk
            .call(&me, &main_branch(), &t.id, at(2, 2))
            .await
            .expect("call");
        if end != "skip" {
            q.desk
                .start(&me, &main_branch(), &t.id, at(2, 3))
                .await
                .expect("start");
        }
        assert!(
            refused(try_close(&db, &q, &a, at(2, 4)).await).starts_with(&still_serving()),
            "{end}: refused while the ticket is live"
        );
        match end {
            "finish" => q.desk.finish(&me, &main_branch(), &t.id, at(2, 5)).await,
            "release" => q.desk.release(&me, &main_branch(), &t.id, at(2, 5)).await,
            _ => q.desk.skip(&me, &main_branch(), &t.id, at(2, 5)).await,
        }
        .unwrap_or_else(|e| panic!("{end}: {e:?}"));

        try_close(&db, &q, &a, at(3, 0)).await.unwrap_or_else(|e| {
            panic!("{end}: the close goes through once the ticket ended: {e:?}")
        });
        assert_eq!(shift_status(&db, &a).await, "CLOSED", "{end}");
        assert_eq!(
            counter_of(&db, &a).await,
            "-",
            "{end}: the counter is removed"
        );
        assert_eq!(active_counters(&db, "MAIN").await, 0, "{end}");
        db.finish().await;
    }
}

/// R-5: WAITING tickets and another teller's CALLED or SERVING ticket do not block A's close.
#[tokio::test]
async fn waiting_tickets_and_another_tellers_live_ticket_do_not_block() {
    let (db, q) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let waiting = walk_in(&db, "r5-w", at(2, 1)).await;
    let bs = walk_in(&db, "r5-b", at(2, 1)).await;
    let tb = teller(&b, "MAIN");
    q.desk
        .call(&tb, &main_branch(), &bs.id, at(2, 2))
        .await
        .expect("B calls");
    q.desk
        .start(&tb, &main_branch(), &bs.id, at(2, 3))
        .await
        .expect("B starts");

    try_close(&db, &q, &a, at(3, 0))
        .await
        .unwrap_or_else(|e| panic!("A holds nothing live: {e:?}"));

    assert_eq!(counter_of(&db, &a).await, "-");
    assert_eq!(counter_of(&db, &b).await, "ACTIVE");
    assert_eq!(
        stored(&db, &waiting.id).await,
        format!("WAITING {}", waiting.display_number)
    );
    assert_eq!(
        stored(&db, &bs.id).await,
        format!("SERVING {}", bs.display_number)
    );
    db.finish().await;
}

/// R-6: the refusal is a `QueueError` through `AppError`, `Validation("ticketStillServing")`, which the cash routes map.
#[test]
fn the_refusal_is_validation_ticket_still_serving() {
    // Restated 2026-11-02 by claude/b091f5e0 (§4.7 amended): the variant carries the live tickets; the code is unchanged
    // and the `Validation` message still starts with it, so the cash routes' prefix match still routes it.
    assert_eq!(
        QueueError::TicketStillServing(Vec::new()).code(),
        "ticketStillServing"
    );
    assert!(matches!(
        AppError::from(QueueError::TicketStillServing(Vec::new())),
        AppError::Validation(c) if c.starts_with("ticketStillServing")
    ));
}
