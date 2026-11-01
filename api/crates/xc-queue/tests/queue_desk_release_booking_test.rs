//! Triage T1 (2026-11-01 real-staff e2e, step 4, booking `B261101-0009`): `QueueDesk::release_in` ends a SERVING
//! BOOKING ticket as CANCELLED `noSale` but never tells the booking, so the booking stays `IN_SERVICE` with no way
//! out. `release_in` must tell `BookingQueueEvents::on_ticket_ended_in` with the reason `noSale`, as the day close
//! does with `dayClosed` (`queue_day_hook.rs:131-133`).
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Pinned here:
//! - releasing a BOOKING ticket calls the port once with the ticket's branch, its `booking_id`, the reason `noSale`
//!   and the command's `now`, after the ticket's `UPDATE` and its `CANCEL` event, inside the desk's transaction;
//! - releasing a walk-in (no booking) does not call `on_ticket_ended_in`;
//! - an error from the port is the command's answer and rolls the release back (the ticket stays SERVING);
//! - a manual `finish` of a BOOKING ticket that no COMPLETED sale names tells the port `noSale` the same way (the
//!   e2e's one-click "เสร็จสิ้น" on B003); a walk-in finish tells nothing. A sale's own completion finishes its
//!   ticket through `WalkInDesk::finish_for_sale_in`, which this does not touch.
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
use xc_kernel::booking_queue_events::BookingQueueEvents;
use xc_kernel::ids::BranchId;
use xc_queue::application::desk::queue_desk::QueueDesk;
use xc_queue::domain::ticket::issue_source::IssueSource;

/// One `on_ticket_ended_in` call, with what the port saw of the booking's ticket inside the desk's transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ended {
    branch: String,
    booking_id: String,
    reason: String,
    now: DateTime<Utc>,
    /// `status | ended_reason | kinds of the ticket's events` read through the desk's `tx`.
    seen: String,
}

/// Records every `on_ticket_ended_in` call; answers `Err` from it when `fails`. `on_ticket_serving_in` is a no-op.
#[derive(Default)]
struct FakeBooking {
    ended: Mutex<Vec<Ended>>,
    fails: bool,
}

impl FakeBooking {
    fn failing() -> Self {
        Self {
            fails: true,
            ..Self::default()
        }
    }

    fn ended(&self) -> Vec<Ended> {
        self.ended.lock().expect("lock").clone()
    }
}

impl BookingQueueEvents for FakeBooking {
    fn on_ticket_serving_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _booking_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn on_ticket_ended_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        booking_id: &'a str,
        reason: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let seen: String = sqlx::query_scalar(
                "SELECT t.status || ' | ' || coalesce(t.ended_reason, '-') || ' | ' || \
                     (SELECT string_agg(e.kind, ',' ORDER BY e.at, e.created_at) FROM queue_events e \
                      WHERE e.ticket_id = t.id) \
                 FROM queue_tickets t WHERE t.branch_id = $1 AND t.booking_id = $2 \
                 ORDER BY t.sequence DESC LIMIT 1",
            )
            .bind(branch.as_str())
            .bind(booking_id)
            .fetch_one(&mut **tx)
            .await?;
            self.ended.lock().expect("lock").push(Ended {
                branch: branch.as_str().to_owned(),
                booking_id: booking_id.to_owned(),
                reason: reason.to_owned(),
                now,
                seen,
            });
            if self.fails {
                Err(AppError::Conflict("bookingBroke".to_owned()))
            } else {
                Ok(())
            }
        })
    }
}

/// An open MAIN queue, one teller on duty, and a desk told through `booking`.
async fn setup(booking: Arc<FakeBooking>) -> (TestDb, QueueDesk, String) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let q = queue(&db);
    let teller_id = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let desk = q.desk.clone().with_booking_events(booking);
    (db, desk, teller_id)
}

/// Booking `booking` and its BOOKING ticket, called and started by `teller_id`; the ticket id.
async fn serving_booking_ticket(
    db: &TestDb,
    desk: &QueueDesk,
    teller_id: &str,
    booking: &str,
) -> String {
    db.add_booking("MAIN", booking).await;
    let id = db
        .issuer()
        .issue(
            &main_branch(),
            IssueSource::Booking {
                booking_id: booking.to_owned(),
            },
            &format!("check-in:{booking}"),
            at(3, 0),
        )
        .await
        .unwrap_or_else(|e| panic!("issue the ticket of {booking}: {e:?}"))
        .id;
    let t = teller(teller_id, "MAIN");
    desk.call(&t, &main_branch(), &id, at(3, 5))
        .await
        .expect("call the booking ticket");
    desk.start(&t, &main_branch(), &id, at(3, 6))
        .await
        .expect("start the booking ticket");
    id
}

/// A desk walk-in, called and started by `teller_id`; the ticket id.
async fn serving_walk_in(db: &TestDb, desk: &QueueDesk, teller_id: &str, key: &str) -> String {
    let id = db
        .issuer()
        .issue(&main_branch(), IssueSource::Desk, key, at(3, 0))
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
        .id;
    let t = teller(teller_id, "MAIN");
    desk.call(&t, &main_branch(), &id, at(3, 5))
        .await
        .expect("call the walk-in");
    desk.start(&t, &main_branch(), &id, at(3, 6))
        .await
        .expect("start the walk-in");
    id
}

#[tokio::test]
async fn t1_releasing_a_booking_ticket_tells_the_booking_no_sale_after_the_ticket_moved() {
    let booking = Arc::new(FakeBooking::default());
    let (db, desk, teller_id) = setup(booking.clone()).await;
    let ticket = serving_booking_ticket(&db, &desk, &teller_id, "BK-T1-1").await;

    let released = desk
        .release(
            &teller(&teller_id, "MAIN"),
            &main_branch(),
            &ticket,
            at(3, 20),
        )
        .await
        .expect("the serving teller releases the booking ticket");

    assert_eq!(released.booking_id.as_deref(), Some("BK-T1-1"));
    assert_eq!(
        booking.ended(),
        vec![Ended {
            branch: "MAIN".to_owned(),
            booking_id: "BK-T1-1".to_owned(),
            reason: "noSale".to_owned(),
            now: at(3, 20),
            seen: "CANCELLED | noSale | ISSUE,CALL,START,CANCEL".to_owned(),
        }],
        "told once, with noSale, after the ticket's UPDATE and CANCEL event"
    );
    db.finish().await;
}

#[tokio::test]
async fn t1_releasing_a_walk_in_does_not_tell_the_booking() {
    let booking = Arc::new(FakeBooking::default());
    let (db, desk, teller_id) = setup(booking.clone()).await;
    let ticket = serving_walk_in(&db, &desk, &teller_id, "desk-1").await;

    desk.release(
        &teller(&teller_id, "MAIN"),
        &main_branch(),
        &ticket,
        at(3, 20),
    )
    .await
    .expect("the serving teller releases the walk-in");

    assert_eq!(booking.ended(), vec![], "a walk-in has no booking to tell");
    db.finish().await;
}

#[tokio::test]
async fn t1_a_booking_port_error_rolls_the_release_back() {
    let booking = Arc::new(FakeBooking::failing());
    let (db, desk, teller_id) = setup(booking.clone()).await;
    let ticket = serving_booking_ticket(&db, &desk, &teller_id, "BK-T1-3").await;

    let res = desk
        .release(
            &teller(&teller_id, "MAIN"),
            &main_branch(),
            &ticket,
            at(3, 20),
        )
        .await;

    assert!(
        matches!(res, Err(AppError::Conflict(ref code)) if code == "bookingBroke"),
        "the port's error is the command's answer, got {res:?}"
    );
    assert_eq!(
        stored(&db, &ticket).await.split(' ').next(),
        Some("SERVING"),
        "the release rolled back"
    );
    let kinds: Vec<String> = events(&db, &ticket)
        .await
        .into_iter()
        .map(|e| e.split(' ').next().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(kinds, ["ISSUE", "CALL", "START"], "no CANCEL event kept");
    db.finish().await;
}

#[tokio::test]
async fn t1_finishing_a_booking_ticket_with_no_sale_tells_the_booking_no_sale() {
    let booking = Arc::new(FakeBooking::default());
    let (db, desk, teller_id) = setup(booking.clone()).await;
    let ticket = serving_booking_ticket(&db, &desk, &teller_id, "BK-T1-4").await;

    desk.finish(
        &teller(&teller_id, "MAIN"),
        &main_branch(),
        &ticket,
        at(3, 20),
    )
    .await
    .expect("the serving teller finishes the booking ticket");

    assert_eq!(
        booking.ended(),
        vec![Ended {
            branch: "MAIN".to_owned(),
            booking_id: "BK-T1-4".to_owned(),
            reason: "noSale".to_owned(),
            now: at(3, 20),
            seen: "COMPLETED | - | ISSUE,CALL,START,FINISH".to_owned(),
        }],
        "no sale names the ticket, so the booking is told noSale after the ticket's UPDATE and FINISH event"
    );
    db.finish().await;
}

#[tokio::test]
async fn t1_finishing_a_walk_in_does_not_tell_the_booking() {
    let booking = Arc::new(FakeBooking::default());
    let (db, desk, teller_id) = setup(booking.clone()).await;
    let ticket = serving_walk_in(&db, &desk, &teller_id, "desk-2").await;

    desk.finish(
        &teller(&teller_id, "MAIN"),
        &main_branch(),
        &ticket,
        at(3, 20),
    )
    .await
    .expect("the serving teller finishes the walk-in");

    assert_eq!(booking.ended(), vec![], "a walk-in has no booking to tell");
    db.finish().await;
}
