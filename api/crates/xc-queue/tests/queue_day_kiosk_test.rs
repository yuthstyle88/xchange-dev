//! #4147 KQ: `QueueDayHook::on_day_closed` runs the kiosk's day-close step first. Design
//! `docs/design/blueprint/queue-day-close.md` §7.2 rows D4–D7 and §7.3: the step (`KioskDayClose`)
//! is called once, on the hook's transaction, **before** the hook's own WAITING/CALLED sweep; a
//! ticket the step ended is not touched again; the step's error rolls the day close back.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-9). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! A fake step stands in for the kiosk: it records what it saw and, like the real one, ends the
//! kiosk-linked ticket as `CANCELLED kioskCancelled:DAY_CLOSED` with one CANCEL event.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use queue_common::*;
use queue_desk_common::*;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::kiosk_day_close::KioskDayClose;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;

/// What the step saw: (branch, open WAITING/CALLED tickets of the branch on the hook's transaction).
#[derive(Default)]
struct FakeKiosk {
    calls: Mutex<Vec<(String, i64)>>,
    fail: bool,
}

impl FakeKiosk {
    fn calls(&self) -> Vec<(String, i64)> {
        self.calls.lock().expect("lock").clone()
    }
}

impl KioskDayClose for FakeKiosk {
    fn close_day_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Vec<String>>> {
        Box::pin(async move {
            let open: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM queue_tickets WHERE branch_id = $1 AND status IN ('WAITING', 'CALLED')",
            )
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await?;
            self.calls
                .lock()
                .expect("lock")
                .push((branch.as_str().to_owned(), open));
            if self.fail {
                return Err(AppError::Internal("the kiosk step failed".into()));
            }
            // As the real step does through `cancel_active_for_kiosk_in`: end the linked ticket.
            let ended: Vec<(String, String)> = sqlx::query_as(
                "UPDATE queue_tickets SET status = 'CANCELLED', ended_reason = 'kioskCancelled:DAY_CLOSED' \
                 WHERE branch_id = $1 AND kiosk_request_id IS NOT NULL AND status IN ('WAITING', 'CALLED', 'SERVING') \
                 RETURNING id::text, organization_id::text",
            )
            .bind(branch.as_str())
            .fetch_all(&mut **tx)
            .await?;
            let mut ids = Vec::new();
            for (ticket, org) in ended {
                sqlx::query(
                    "INSERT INTO queue_events (id, organization_id, ticket_id, status, kind, actor_id, at, reason) \
                     VALUES (gen_random_uuid(), $1::uuid, $2::uuid, 'CANCELLED', 'CANCEL', NULL, $3, 'kioskCancelled:DAY_CLOSED')",
                )
                .bind(&org)
                .bind(&ticket)
                .bind(now)
                .execute(&mut **tx)
                .await?;
                ids.push(ticket);
            }
            Ok(ids)
        })
    }
}

async fn issue(db: &TestDb, source: IssueSource, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&BranchId::new("MAIN"), source, key, now)
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

async fn cancel_events(db: &TestDb, id: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM queue_events WHERE ticket_id = '{id}'::uuid AND kind = 'CANCEL'"
    ))
    .await
}

/// MAIN with a desk walk-in WAITING ticket and a kiosk-linked WAITING ticket.
async fn board(db: &TestDb) -> (QueueTicket, QueueTicket) {
    db.open_queue("MAIN").await;
    let walk_in = issue(db, IssueSource::Desk, "kq-desk", at(3, 0)).await;
    db.add_kiosk_request("MAIN", "KR-KQ").await;
    let kiosk = issue(
        db,
        IssueSource::Kiosk {
            request_id: "KR-KQ".to_owned(),
        },
        "kiosk-confirm:KR-KQ",
        at(3, 1),
    )
    .await;
    (walk_in, kiosk)
}

async fn close_day(
    db: &TestDb,
    q: &Queue,
    kiosk: Option<Arc<FakeKiosk>>,
) -> AppResult<Vec<QueueTicket>> {
    let mut day = q.day.clone();
    if let Some(step) = kiosk {
        day = day.with_kiosk_day_close(step);
    }
    let b = BranchId::new("MAIN");
    db.branch_tx()
        .run(&BranchId::new("MAIN"), move |tx| {
            Box::pin(async move { day.on_day_closed(tx, &b, at(12, 0)).await })
        })
        .await
}

/// D7: the step runs first — it sees both tickets still WAITING — and the ticket it ended keeps its
/// `kioskCancelled:DAY_CLOSED` with exactly one CANCEL event; D4: the desk ticket gets `dayClosed`.
#[tokio::test]
async fn d4_d7_the_kiosk_step_runs_first_and_its_ticket_is_not_touched_again() {
    let db = TestDb::start().await;
    let q = queue(&db);
    let (walk_in, kiosk) = board(&db).await;
    let step = Arc::new(FakeKiosk::default());

    let cancelled = close_day(&db, &q, Some(step.clone()))
        .await
        .expect("on_day_closed");

    assert_eq!(
        step.calls(),
        vec![("MAIN".to_owned(), 2)],
        "called once, before the hook's own sweep"
    );
    assert_eq!(
        ended(&db, &kiosk.id).await,
        "CANCELLED kioskCancelled:DAY_CLOSED"
    );
    assert_eq!(
        cancel_events(&db, &kiosk.id).await,
        1,
        "no second CANCEL event"
    );
    assert_eq!(ended(&db, &walk_in.id).await, "CANCELLED dayClosed");
    assert_eq!(cancel_events(&db, &walk_in.id).await, 1);
    let ids: Vec<String> = cancelled.into_iter().map(|t| t.id).collect();
    assert_eq!(
        ids,
        vec![walk_in.id.clone()],
        "the hook answers only what it cancelled itself"
    );
}

/// D5: the step's error is the day close's error; the caller's transaction rolls back and nothing moved.
#[tokio::test]
async fn d5_the_kiosk_steps_error_rolls_the_day_close_back() {
    let db = TestDb::start().await;
    let q = queue(&db);
    let (walk_in, kiosk) = board(&db).await;
    let step = Arc::new(FakeKiosk {
        calls: Mutex::default(),
        fail: true,
    });

    let answer = close_day(&db, &q, Some(step.clone())).await;

    assert!(answer.is_err(), "the step's error is answered");
    assert_eq!(step.calls().len(), 1);
    assert_eq!(ended(&db, &walk_in.id).await, "WAITING -");
    assert_eq!(ended(&db, &kiosk.id).await, "WAITING -");
}

/// D6: with no step installed the hook behaves as in §2: the kiosk-linked ticket gets `dayClosed` too.
#[tokio::test]
async fn d6_without_a_step_the_hook_behaves_as_before() {
    let db = TestDb::start().await;
    let q = queue(&db);
    let (walk_in, kiosk) = board(&db).await;

    close_day(&db, &q, None).await.expect("on_day_closed");

    assert_eq!(ended(&db, &walk_in.id).await, "CANCELLED dayClosed");
    assert_eq!(ended(&db, &kiosk.id).await, "CANCELLED dayClosed");
}
