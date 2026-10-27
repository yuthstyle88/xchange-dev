//! XC-BE-GAPS-A row 6 (queue side): `QueueDesk::skip_in` / `cancel_in` tell `KioskQueueEvents` when staff end a
//! kiosk-linked ticket, so the request it came from does not stay `WAITING` behind an ended ticket.
//!
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 6, 2026-10-27).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/kiosk-cancel.md:207` (§3.7: "A staff `ticketCancel` / skip on a kiosk-linked
//! ticket should also cancel the request … needs a queue → kiosk hook"); the port is
//! `xc_kernel::kiosk_queue_events::KioskQueueEvents`, set with `QueueDesk::with_kiosk_events`.
//!
//! Pinned here:
//! - skip of a kiosk-linked ticket calls the port once with the ticket's branch, its `kiosk_request_id`,
//!   `KioskTicketEnd::Skipped`, the caller's `user_id` and the command's `now`; cancel calls it with
//!   `Cancelled { reason }`, the reason exactly as the desk stored it;
//! - the call comes after the ticket's `UPDATE` and its event, inside the desk's transaction (the port sees the
//!   ticket already `NO_SHOW` / `CANCELLED` with its `SKIP` / `CANCEL` event);
//! - a ticket with no kiosk request does not call the port;
//! - an error from the port is the command's answer and rolls the queue action back (ticket and events
//!   unchanged).
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
use xc_kernel::kiosk_queue_events::{KioskQueueEvents, KioskTicketEnd};
use xc_queue::application::desk::queue_desk::QueueDesk;
use xc_queue::domain::ticket::issue_source::IssueSource;

/// One call of the port, with what the port saw of the ticket inside the desk's transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Told {
    branch: String,
    request_id: String,
    end: KioskTicketEnd,
    actor_id: String,
    now: DateTime<Utc>,
    /// `status | ended_reason | kinds of the ticket's events` read through the desk's `tx`.
    seen: String,
}

/// Records every call; answers `Err` when `fails`.
#[derive(Default)]
struct FakeKiosk {
    told: Mutex<Vec<Told>>,
    fails: bool,
}

impl FakeKiosk {
    fn failing() -> Self {
        Self {
            fails: true,
            ..Self::default()
        }
    }

    fn told(&self) -> Vec<Told> {
        self.told.lock().expect("lock").clone()
    }
}

impl KioskQueueEvents for FakeKiosk {
    fn on_ticket_ended_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
        end: &'a KioskTicketEnd,
        actor_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let seen: String = sqlx::query_scalar(
                "SELECT t.status || ' | ' || coalesce(t.ended_reason, '-') || ' | ' || \
                     (SELECT string_agg(e.kind, ',' ORDER BY e.at, e.created_at) FROM queue_events e \
                      WHERE e.ticket_id = t.id) \
                 FROM queue_tickets t WHERE t.branch_id = $1 AND t.kiosk_request_id = $2",
            )
            .bind(branch.as_str())
            .bind(request_id)
            .fetch_one(&mut **tx)
            .await?;
            self.told.lock().expect("lock").push(Told {
                branch: branch.as_str().to_owned(),
                request_id: request_id.to_owned(),
                end: end.clone(),
                actor_id: actor_id.to_owned(),
                now,
                seen,
            });
            if self.fails {
                Err(AppError::Conflict("kioskBroke".to_owned()))
            } else {
                Ok(())
            }
        })
    }
}

/// An open MAIN queue, one teller on duty, and a desk told through `kiosk`.
async fn setup(kiosk: Arc<FakeKiosk>) -> (TestDb, QueueDesk, String) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let q = queue(&db);
    let teller_id = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let desk = q.desk.clone().with_kiosk_events(kiosk);
    (db, desk, teller_id)
}

/// Kiosk request `request` (WAITING) and its walk-in ticket; the ticket id.
async fn kiosk_ticket(db: &TestDb, request: &str, now: DateTime<Utc>) -> String {
    db.add_kiosk_request("MAIN", request).await;
    db.issuer()
        .issue(
            &main_branch(),
            IssueSource::Kiosk {
                request_id: request.to_owned(),
            },
            &format!("kiosk-confirm:{request}"),
            now,
        )
        .await
        .unwrap_or_else(|e| panic!("issue the ticket of {request}: {e:?}"))
        .id
}

/// A desk walk-in with no kiosk request; the ticket id.
async fn desk_ticket(db: &TestDb, key: &str, now: DateTime<Utc>) -> String {
    db.issuer()
        .issue(&main_branch(), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
        .id
}

async fn kinds(db: &TestDb, ticket: &str) -> Vec<String> {
    events(db, ticket)
        .await
        .into_iter()
        .map(|e| e.split(' ').next().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn gapsa6_a_skip_of_a_kiosk_ticket_tells_the_kiosk_after_the_ticket_moved() {
    let kiosk = Arc::new(FakeKiosk::default());
    let (db, desk, teller_id) = setup(kiosk.clone()).await;
    let plain = desk_ticket(&db, "desk-1", at(3, 0)).await;
    let linked = kiosk_ticket(&db, "KR-1", at(3, 1)).await;
    let t = teller(&teller_id, "MAIN");

    desk.call(&t, &main_branch(), &plain, at(3, 5))
        .await
        .expect("call the desk ticket");
    desk.skip(&t, &main_branch(), &plain, at(3, 6))
        .await
        .expect("skip the desk ticket");
    desk.call(&t, &main_branch(), &linked, at(3, 7))
        .await
        .expect("call the kiosk ticket");
    let skipped = desk
        .skip(&t, &main_branch(), &linked, at(3, 8))
        .await
        .expect("skip the kiosk ticket");

    assert_eq!(skipped.kiosk_request_id.as_deref(), Some("KR-1"));
    assert_eq!(
        kiosk.told(),
        vec![Told {
            branch: "MAIN".to_owned(),
            request_id: "KR-1".to_owned(),
            end: KioskTicketEnd::Skipped,
            actor_id: teller_id.clone(),
            now: at(3, 8),
            seen: "NO_SHOW | - | ISSUE,CALL,SKIP".to_owned(),
        }],
        "told once, for the kiosk ticket only, after its UPDATE and SKIP event"
    );
    assert_eq!(kinds(&db, &linked).await, ["ISSUE", "CALL", "SKIP"]);
    db.finish().await;
}

#[tokio::test]
async fn gapsa6_a_cancel_of_a_kiosk_ticket_tells_the_kiosk_the_reason_as_stored() {
    let kiosk = Arc::new(FakeKiosk::default());
    let (db, desk, teller_id) = setup(kiosk.clone()).await;
    let plain = desk_ticket(&db, "desk-1", at(3, 0)).await;
    let waiting = kiosk_ticket(&db, "KR-W", at(3, 1)).await;
    let called = kiosk_ticket(&db, "KR-C", at(3, 2)).await;
    let t = teller(&teller_id, "MAIN");

    desk.cancel(&t, &main_branch(), &plain, "DUPLICATE", at(3, 5))
        .await
        .expect("cancel the desk ticket");
    desk.cancel(&t, &main_branch(), &waiting, "DUPLICATE", at(3, 6))
        .await
        .expect("cancel the waiting kiosk ticket");
    desk.call(&t, &main_branch(), &called, at(3, 7))
        .await
        .expect("call");
    desk.cancel(&t, &main_branch(), &called, "ลูกค้าไปธนาคารอื่น", at(3, 8))
        .await
        .expect("cancel the called kiosk ticket");

    assert_eq!(
        kiosk.told(),
        vec![
            Told {
                branch: "MAIN".to_owned(),
                request_id: "KR-W".to_owned(),
                end: KioskTicketEnd::Cancelled {
                    reason: "DUPLICATE".to_owned()
                },
                actor_id: teller_id.clone(),
                now: at(3, 6),
                seen: "CANCELLED | DUPLICATE | ISSUE,CANCEL".to_owned(),
            },
            Told {
                branch: "MAIN".to_owned(),
                request_id: "KR-C".to_owned(),
                end: KioskTicketEnd::Cancelled {
                    reason: "ลูกค้าไปธนาคารอื่น".to_owned()
                },
                actor_id: teller_id.clone(),
                now: at(3, 8),
                seen: "CANCELLED | ลูกค้าไปธนาคารอื่น | ISSUE,CALL,CANCEL".to_owned(),
            },
        ]
    );
    db.finish().await;
}

#[tokio::test]
async fn gapsa6_a_kiosk_error_rolls_the_skip_back() {
    let kiosk = Arc::new(FakeKiosk::failing());
    let (db, desk, teller_id) = setup(kiosk.clone()).await;
    let linked = kiosk_ticket(&db, "KR-1", at(3, 0)).await;
    let t = teller(&teller_id, "MAIN");
    desk.call(&t, &main_branch(), &linked, at(3, 5))
        .await
        .expect("call");

    let answer = desk.skip(&t, &main_branch(), &linked, at(3, 6)).await;

    assert_eq!(
        refused(answer),
        "kioskBroke",
        "the port's error is the answer"
    );
    assert_eq!(kiosk.told().len(), 1, "the port was asked once");
    assert!(
        stored(&db, &linked).await.starts_with("CALLED "),
        "the skip is rolled back: {}",
        stored(&db, &linked).await
    );
    assert_eq!(kinds(&db, &linked).await, ["ISSUE", "CALL"]);
    db.finish().await;
}

#[tokio::test]
async fn gapsa6_a_kiosk_error_rolls_the_cancel_back() {
    let kiosk = Arc::new(FakeKiosk::failing());
    let (db, desk, teller_id) = setup(kiosk.clone()).await;
    let linked = kiosk_ticket(&db, "KR-1", at(3, 0)).await;
    let t = teller(&teller_id, "MAIN");

    let answer = desk
        .cancel(&t, &main_branch(), &linked, "DUPLICATE", at(3, 6))
        .await;

    assert_eq!(
        refused(answer),
        "kioskBroke",
        "the port's error is the answer"
    );
    assert_eq!(kiosk.told().len(), 1, "the port was asked once");
    assert!(
        stored(&db, &linked).await.starts_with("WAITING "),
        "the cancel is rolled back: {}",
        stored(&db, &linked).await
    );
    let ended: Option<String> = db
        .text(&format!(
            "SELECT ended_reason FROM queue_tickets WHERE id = '{linked}'::uuid"
        ))
        .await;
    assert_eq!(ended, None);
    assert_eq!(kinds(&db, &linked).await, ["ISSUE"]);
    db.finish().await;
}
