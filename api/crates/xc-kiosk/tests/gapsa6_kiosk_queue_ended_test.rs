//! XC-BE-GAPS-A row 6 (kiosk side): `KioskQueueEnded` — a staff skip or cancel of a kiosk-linked queue
//! ticket closes the kiosk request it came from.
//!
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 6, 2026-10-27).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/kiosk-cancel.md:207` (§3.7: a staff `ticketCancel` / skip on a kiosk-linked
//! ticket also cancels the request, through a queue → kiosk hook), the reason list `:36-48`, §2's table `:27-35`
//! (skip is the queue's `NO_SHOW`, which the kiosk records as `CUSTOMER_LEFT`), §3.3 (`cancel_in`).
//!
//! Pinned here:
//! - `KioskQueueEnded::reason_for`: `Skipped` ⇒ `CustomerLeft { note: None }`; a `Cancelled` reason that is a
//!   staff code (`CUSTOMER_LEFT`, `CUSTOMER_CHANGED_MIND`, `DUPLICATE`) ⇒ that code with no note; anything else
//!   (free text, `OTHER`, `CUSTOMER_CANCELLED`, `DAY_CLOSED`, a code in another spelling, padded text) ⇒
//!   `Other { note: <the reason, trimmed> }`. `CUSTOMER_CANCELLED` and `DAY_CLOSED` are not staff codes (§2:
//!   "count as unknown when staff send them").
//! - `on_ticket_ended_in` cancels through the same `cancel_in` as a staff cancel (the model is
//!   `day_close.rs`): the row (`cancelled_by` = the staff id), one `KIOSK_CANCELLED` line naming the staff, and
//!   the queue port asked to end the ticket. The actor is a manager, so a request held by another teller is
//!   still cancelled (the queue already authorised the caller to end the ticket).
//! - A request that is already ended, never confirmed, or unknown is left as it is and the answer is `Ok(())`.
//! - A failure inside `cancel_in` (here: the audit line) is answered as an error, so the queue action rolls back.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test` (the `reason_for` tests do
//! not).

mod kiosk_service_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use kiosk_service_common::*;
use pos_error::AppResult;
use sqlx::AssertSqlSafe;
use xc_kernel::ids::BranchId;
use xc_kernel::kiosk_queue_events::{KioskQueueEvents, KioskTicketEnd};
use xc_kernel::queue_tickets::{IssuedTicket, KioskTicketOutcome, QueueTicketSource, QueueTickets};
use xc_kiosk::application::kiosk::queue_ended::KioskQueueEnded;
use xc_kiosk::domain::kiosk::value_objects::cancel_reason::CancelReason;
use xc_kiosk::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

const TELLER: &str = "00000000-0000-0000-0000-0000000000a1";

// ------------------------------------------------------------ reason_for

fn cancelled(reason: &str) -> KioskTicketEnd {
    KioskTicketEnd::Cancelled {
        reason: reason.to_owned(),
    }
}

#[test]
fn gapsa6_a_skip_is_customer_left_with_no_note() {
    assert_eq!(
        KioskQueueEnded::reason_for(&KioskTicketEnd::Skipped),
        CancelReason::CustomerLeft { note: None }
    );
}

#[test]
fn gapsa6_a_cancel_with_a_staff_code_keeps_that_code_with_no_note() {
    assert_eq!(
        KioskQueueEnded::reason_for(&cancelled("CUSTOMER_LEFT")),
        CancelReason::CustomerLeft { note: None }
    );
    assert_eq!(
        KioskQueueEnded::reason_for(&cancelled("CUSTOMER_CHANGED_MIND")),
        CancelReason::CustomerChangedMind { note: None }
    );
    assert_eq!(
        KioskQueueEnded::reason_for(&cancelled("DUPLICATE")),
        CancelReason::Duplicate { note: None }
    );
}

#[test]
fn gapsa6_any_other_cancel_reason_is_other_with_the_trimmed_reason_as_its_note() {
    for (reason, note) in [
        ("ลูกค้าโทรมาแจ้งว่าไม่มาแล้ว", "ลูกค้าโทรมาแจ้งว่าไม่มาแล้ว"),
        ("OTHER", "OTHER"),
        ("CUSTOMER_CANCELLED", "CUSTOMER_CANCELLED"),
        ("DAY_CLOSED", "DAY_CLOSED"),
        ("customerLeft", "customerLeft"),
        ("duplicate", "duplicate"),
        ("  ลูกค้าไปธนาคารอื่น \t", "ลูกค้าไปธนาคารอื่น"),
    ] {
        assert_eq!(
            KioskQueueEnded::reason_for(&cancelled(reason)),
            CancelReason::Other {
                note: note.to_owned()
            },
            "reason {reason:?}"
        );
    }
}

// ------------------------------------------------------------ on_ticket_ended_in

/// Records every `cancel_active_for_kiosk_in` (request id, outcome); every other method is unused.
#[derive(Default)]
struct FakeQueue {
    ended: Mutex<Vec<(String, KioskTicketOutcome)>>,
}

impl FakeQueue {
    fn ended(&self) -> Vec<(String, KioskTicketOutcome)> {
        self.ended.lock().expect("lock").clone()
    }
}

impl QueueTickets for FakeQueue {
    fn issue_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _source: &'a QueueTicketSource,
        _key: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<IssuedTicket>> {
        unreachable!("ending a ticket issues none")
    }

    fn start_for_kiosk_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _request_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        unreachable!("ending a ticket starts none")
    }

    fn cancel_active_for_booking_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _booking_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        unreachable!("no booking ticket here")
    }

    fn cancel_active_for_kiosk_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        request_id: &'a str,
        outcome: &'a KioskTicketOutcome,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            self.ended
                .lock()
                .expect("lock")
                .push((request_id.to_owned(), outcome.clone()));
            // The queue already ended the ticket itself: nothing left to end.
            Ok(false)
        })
    }
}

/// A request of `branch` stored in `status`, held by `staff`.
async fn add(db: &TestDb, branch: &str, id: &str, status: &str, staff: Option<&str>) {
    let staff = staff.map_or("NULL".to_owned(), |s| format!("'{s}'"));
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, rate, \
             foreign_amount, thb, input_amount, payment_method) \
         VALUES ('{branch}', '{id}', '{status}', now(), {staff}, 'BUY', 'USD', 35.0000, 100.00, \
             3500.00, 100.00, 'Cash')"
    )))
    .execute(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("add {id} {status}: {e}"));
}

/// `status | staff | cancelled_by | cancel_reason | cancel_note` of one row.
async fn row(db: &TestDb, branch: &str, id: &str) -> String {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT concat_ws(' | ', status, coalesce(staff, '-'), coalesce(cancelled_by, '-'), \
             coalesce(cancel_reason, '-'), coalesce(cancel_note, '-')) \
         FROM kiosk_requests WHERE branch_id = '{branch}' AND id = '{id}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("read the row")
}

/// `actor | detail` of every `KIOSK_CANCELLED` line of a request.
async fn cancel_lines(db: &TestDb, id: &str) -> Vec<String> {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT actor || ' | ' || detail FROM test_audit WHERE kind = 'KIOSK_CANCELLED' AND subject = '{id}' ORDER BY seq"
    )))
    .fetch_all(&db.pool)
    .await
    .expect("read the audit")
}

/// Runs `on_ticket_ended_in` in a transaction of its own, committed when it answers `Ok`.
async fn ended(
    db: &TestDb,
    ports: Arc<FakePorts>,
    queue: Arc<FakeQueue>,
    branch: &str,
    request: &str,
    end: KioskTicketEnd,
    actor: &str,
) -> AppResult<()> {
    let hook = KioskQueueEnded::new(PgKioskRepository::new(db.pool.clone()), ports, queue);
    let mut tx = db.begin().await;
    let answer = hook
        .on_ticket_ended_in(&mut tx, &BranchId::new(branch), request, &end, actor, at(0))
        .await;
    if answer.is_ok() {
        tx.commit().await.expect("commit");
    }
    answer
}

#[tokio::test]
async fn gapsa6_a_skip_cancels_the_waiting_request_customer_left_by_the_staff() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-S", "WAITING", None).await;
    let queue = Arc::new(FakeQueue::default());

    ended(
        &db,
        FakePorts::standard(),
        queue.clone(),
        MAIN,
        "KR-S",
        KioskTicketEnd::Skipped,
        TELLER,
    )
    .await
    .expect("on_ticket_ended_in");

    assert_eq!(
        row(&db, MAIN, "KR-S").await,
        format!("CANCELLED | - | {TELLER} | CUSTOMER_LEFT | -")
    );
    let lines = cancel_lines(&db, "KR-S").await;
    assert_eq!(lines.len(), 1, "one KIOSK_CANCELLED line: {lines:?}");
    assert!(
        lines[0].starts_with(&format!("{TELLER} | "))
            && lines[0].contains("\"from\":\"WAITING\"")
            && lines[0].contains("\"reason\":\"CUSTOMER_LEFT\""),
        "{}",
        lines[0]
    );
    assert_eq!(
        queue.ended(),
        vec![("KR-S".to_owned(), KioskTicketOutcome::NoShow)]
    );
    db.finish().await;
}

#[tokio::test]
async fn gapsa6_a_cancel_with_free_text_cancels_the_request_other_with_the_text_as_its_note() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-T", "WAITING", None).await;
    add(&db, MAIN, "KR-D", "WAITING", None).await;
    let queue = Arc::new(FakeQueue::default());
    let ports = FakePorts::standard();

    ended(
        &db,
        ports.clone(),
        queue.clone(),
        MAIN,
        "KR-T",
        cancelled(" ลูกค้าจะกลับมาพรุ่งนี้ "),
        TELLER,
    )
    .await
    .expect("free text");
    ended(
        &db,
        ports,
        queue.clone(),
        MAIN,
        "KR-D",
        cancelled("DUPLICATE"),
        TELLER,
    )
    .await
    .expect("DUPLICATE");

    assert_eq!(
        row(&db, MAIN, "KR-T").await,
        format!("CANCELLED | - | {TELLER} | OTHER | ลูกค้าจะกลับมาพรุ่งนี้")
    );
    assert_eq!(
        row(&db, MAIN, "KR-D").await,
        format!("CANCELLED | - | {TELLER} | DUPLICATE | -")
    );
    assert_eq!(cancel_lines(&db, "KR-T").await.len(), 1);
    assert_eq!(cancel_lines(&db, "KR-D").await.len(), 1);
    assert_eq!(
        queue.ended(),
        vec![
            (
                "KR-T".to_owned(),
                KioskTicketOutcome::Cancelled {
                    reason: "OTHER".to_owned()
                }
            ),
            (
                "KR-D".to_owned(),
                KioskTicketOutcome::Cancelled {
                    reason: "DUPLICATE".to_owned()
                }
            ),
        ]
    );
    db.finish().await;
}

/// The queue already authorised the caller to end the ticket; the kiosk adds no holder check of its own.
#[tokio::test]
async fn gapsa6_a_request_held_by_another_teller_is_still_cancelled() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-H", "ACCEPTED", Some("front2")).await;
    let queue = Arc::new(FakeQueue::default());

    ended(
        &db,
        FakePorts::standard(),
        queue.clone(),
        MAIN,
        "KR-H",
        cancelled("CUSTOMER_CHANGED_MIND"),
        TELLER,
    )
    .await
    .expect("on_ticket_ended_in");

    assert_eq!(
        row(&db, MAIN, "KR-H").await,
        format!("CANCELLED | front2 | {TELLER} | CUSTOMER_CHANGED_MIND | -")
    );
    let lines = cancel_lines(&db, "KR-H").await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("\"from\":\"ACCEPTED\""), "{}", lines[0]);
    db.finish().await;
}

/// An ended, unconfirmed or unknown request: `Ok(())`, nothing written, the queue port not asked. A live
/// request beside them is cancelled, so a hook that does nothing at all does not pass.
#[tokio::test]
async fn gapsa6_an_ended_unconfirmed_or_unknown_request_is_left_alone_and_answers_ok() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-C", "COMPLETED", Some("front1")).await;
    add(&db, MAIN, "KR-Q", "QUOTED", None).await;
    add(&db, MAIN, "KR-X", "WAITING", None).await;
    sqlx::query(
        "UPDATE kiosk_requests SET status = 'CANCELLED', cancelled_at = now(), cancelled_by = 'front1', \
         cancel_reason = 'DUPLICATE' WHERE branch_id = 'MAIN' AND id = 'KR-X'",
    )
    .execute(&db.pool)
    .await
    .expect("an already cancelled request");
    add(&db, B2, "KR-OTHER-BRANCH", "WAITING", None).await;
    add(&db, MAIN, "KR-LIVE", "WAITING", None).await;
    let queue = Arc::new(FakeQueue::default());
    let ports = FakePorts::standard();

    for (id, end) in [
        ("KR-C", KioskTicketEnd::Skipped),
        ("KR-C", cancelled("DUPLICATE")),
        ("KR-Q", cancelled("CUSTOMER_LEFT")),
        ("KR-X", KioskTicketEnd::Skipped),
        ("KR-X", cancelled("ลูกค้าไม่มา")),
        ("KR-NONE", KioskTicketEnd::Skipped),
        ("KR-OTHER-BRANCH", KioskTicketEnd::Skipped),
    ] {
        ended(
            &db,
            ports.clone(),
            queue.clone(),
            MAIN,
            id,
            end.clone(),
            TELLER,
        )
        .await
        .unwrap_or_else(|e| panic!("{id} {end:?} answers Ok(()): {e:?}"));
    }
    ended(
        &db,
        ports,
        queue.clone(),
        MAIN,
        "KR-LIVE",
        KioskTicketEnd::Skipped,
        TELLER,
    )
    .await
    .expect("the live request");

    assert_eq!(
        row(&db, MAIN, "KR-C").await,
        "COMPLETED | front1 | - | - | -"
    );
    assert_eq!(row(&db, MAIN, "KR-Q").await, "QUOTED | - | - | - | -");
    assert_eq!(
        row(&db, MAIN, "KR-X").await,
        "CANCELLED | - | front1 | DUPLICATE | -"
    );
    assert_eq!(
        row(&db, B2, "KR-OTHER-BRANCH").await,
        "WAITING | - | - | - | -"
    );
    for id in ["KR-C", "KR-Q", "KR-X", "KR-NONE", "KR-OTHER-BRANCH"] {
        assert!(cancel_lines(&db, id).await.is_empty(), "no line for {id}");
    }
    assert_eq!(
        row(&db, MAIN, "KR-LIVE").await,
        format!("CANCELLED | - | {TELLER} | CUSTOMER_LEFT | -")
    );
    assert_eq!(
        queue.ended(),
        vec![("KR-LIVE".to_owned(), KioskTicketOutcome::NoShow)],
        "the queue port is asked for the live request only"
    );
    db.finish().await;
}

/// A failure inside the cancel is the hook's answer, so the queue's transaction rolls back.
#[tokio::test]
async fn gapsa6_a_failure_inside_the_cancel_is_answered_as_an_error() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-F", "WAITING", None).await;
    let queue = Arc::new(FakeQueue::default());
    let ports = FakePorts::standard();
    ports.audit_fails(true);

    let answer = ended(
        &db,
        ports,
        queue,
        MAIN,
        "KR-F",
        KioskTicketEnd::Skipped,
        TELLER,
    )
    .await;

    assert!(answer.is_err(), "the audit failure is answered: {answer:?}");
    assert_eq!(row(&db, MAIN, "KR-F").await, "WAITING | - | - | - | -");
    db.finish().await;
}
