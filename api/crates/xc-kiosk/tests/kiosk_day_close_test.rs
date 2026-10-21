//! #4147 KX: `KioskDayCloser::close_day_in` against a real PostgreSQL. Design
//! `docs/design/blueprint/queue-day-close.md` §7.2 rows D1–D3 and D8, §7.3 (kiosk side): every
//! unfinished request of the branch (`WAITING`, `ACCEPTED`, `ID_CHECKED`) is cancelled with
//! `DAY_CLOSED` by `SYSTEM-DAY-CLOSE` through `cancel_in` — the row, one `KIOSK_CANCELLED` line,
//! and the queue port told to end its ticket — and nothing else moves.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-9). Do not edit,
//! delete or weaken; add tests in a file of your own. The `ACCEPTED`/`ID_CHECKED` rows need KM
//! (owner ruling K-4).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod kiosk_service_common;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use kiosk_service_common::*;
use pos_error::AppResult;
use sqlx::AssertSqlSafe;
use xc_kernel::ids::BranchId;
use xc_kernel::kiosk_day_close::KioskDayClose;
use xc_kernel::queue_tickets::{IssuedTicket, KioskTicketOutcome, QueueTicketSource, QueueTickets};
use xc_kiosk::application::kiosk::day_close::{DAY_CLOSE_ACTOR, KioskDayCloser};
use xc_kiosk::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

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
        unreachable!("day close issues no ticket")
    }

    fn start_for_kiosk_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _request_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        unreachable!("day close starts no ticket")
    }

    fn cancel_active_for_booking_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _booking_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        unreachable!("day close cancels no booking ticket here")
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
            Ok(true)
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

/// `status | cancelled_by | cancel_reason` of one row.
async fn row(db: &TestDb, branch: &str, id: &str) -> String {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT concat_ws(' | ', status, coalesce(cancelled_by, '-'), coalesce(cancel_reason, '-')) \
         FROM kiosk_requests WHERE branch_id = '{branch}' AND id = '{id}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("read the row")
}

/// `actor | detail` of the `KIOSK_CANCELLED` lines of a request.
async fn cancel_lines(db: &TestDb, id: &str) -> Vec<String> {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT actor || ' | ' || detail FROM test_audit WHERE kind = 'KIOSK_CANCELLED' AND subject = '{id}' ORDER BY seq"
    )))
    .fetch_all(&db.pool)
    .await
    .expect("read the audit")
}

async fn close_day(db: &TestDb, queue: Arc<FakeQueue>, branch: &str) -> AppResult<Vec<String>> {
    let closer = KioskDayCloser::new(
        PgKioskRepository::new(db.pool.clone()),
        FakePorts::standard(),
        queue,
    );
    let mut tx = db.begin().await;
    let answer = closer
        .close_day_in(&mut tx, &BranchId::new(branch), at(0))
        .await;
    if answer.is_ok() {
        tx.commit().await.expect("commit");
    }
    answer
}

fn day_closed() -> KioskTicketOutcome {
    KioskTicketOutcome::Cancelled {
        reason: "DAY_CLOSED".to_owned(),
    }
}

/// D1: a `WAITING` request is cancelled `DAY_CLOSED` by `SYSTEM-DAY-CLOSE`, with one
/// `KIOSK_CANCELLED` line, and its ticket is ended as `Cancelled { DAY_CLOSED }`.
#[tokio::test]
async fn d1_a_waiting_request_is_cancelled_day_closed() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-W", "WAITING", None).await;
    let queue = Arc::new(FakeQueue::default());

    let ids = close_day(&db, queue.clone(), MAIN)
        .await
        .expect("close_day_in");

    assert_eq!(ids, vec!["KR-W".to_owned()]);
    assert_eq!(DAY_CLOSE_ACTOR, "SYSTEM-DAY-CLOSE");
    assert_eq!(
        row(&db, MAIN, "KR-W").await,
        "CANCELLED | SYSTEM-DAY-CLOSE | DAY_CLOSED"
    );
    let lines = cancel_lines(&db, "KR-W").await;
    assert_eq!(lines.len(), 1, "one KIOSK_CANCELLED line: {lines:?}");
    assert!(
        lines[0].starts_with("SYSTEM-DAY-CLOSE | ")
            && lines[0].contains("\"from\":\"WAITING\"")
            && lines[0].contains("\"reason\":\"DAY_CLOSED\""),
        "{}",
        lines[0]
    );
    assert_eq!(queue.ended(), vec![("KR-W".to_owned(), day_closed())]);
}

/// D2 (K-4): `ACCEPTED` and `ID_CHECKED` requests, held by any teller, are cancelled the same way.
#[tokio::test]
async fn d2_held_requests_are_cancelled_day_closed() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-A", "ACCEPTED", Some("front2")).await;
    add(&db, MAIN, "KR-I", "ID_CHECKED", Some("front1")).await;
    let queue = Arc::new(FakeQueue::default());

    let mut ids = close_day(&db, queue.clone(), MAIN)
        .await
        .expect("close_day_in");
    ids.sort();

    assert_eq!(ids, vec!["KR-A".to_owned(), "KR-I".to_owned()]);
    assert_eq!(
        row(&db, MAIN, "KR-A").await,
        "CANCELLED | SYSTEM-DAY-CLOSE | DAY_CLOSED"
    );
    assert_eq!(
        row(&db, MAIN, "KR-I").await,
        "CANCELLED | SYSTEM-DAY-CLOSE | DAY_CLOSED"
    );
    assert!(cancel_lines(&db, "KR-A").await[0].contains("\"from\":\"ACCEPTED\""));
    assert!(cancel_lines(&db, "KR-I").await[0].contains("\"from\":\"ID_CHECKED\""));
    let mut ended = queue.ended();
    ended.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        ended,
        vec![
            ("KR-A".to_owned(), day_closed()),
            ("KR-I".to_owned(), day_closed())
        ]
    );
}

/// D3: `QUOTED`, `COMPLETED` and `CANCELLED` requests are left as they are: no row change, no line,
/// no queue call.
#[tokio::test]
async fn d3_finished_and_quoted_requests_are_left_alone() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-Q", "QUOTED", None).await;
    add(&db, MAIN, "KR-C", "COMPLETED", Some("front1")).await;
    add(&db, MAIN, "KR-X", "WAITING", None).await;
    sqlx::query(
        "UPDATE kiosk_requests SET status = 'CANCELLED', cancelled_at = now(), cancelled_by = 'front1', \
         cancel_reason = 'DUPLICATE' WHERE branch_id = 'MAIN' AND id = 'KR-X'",
    )
    .execute(&db.pool)
    .await
    .expect("an already cancelled request");
    let queue = Arc::new(FakeQueue::default());

    let ids = close_day(&db, queue.clone(), MAIN)
        .await
        .expect("close_day_in");

    assert!(ids.is_empty(), "{ids:?}");
    assert_eq!(row(&db, MAIN, "KR-Q").await, "QUOTED | - | -");
    assert_eq!(row(&db, MAIN, "KR-C").await, "COMPLETED | - | -");
    assert_eq!(
        row(&db, MAIN, "KR-X").await,
        "CANCELLED | front1 | DUPLICATE"
    );
    assert!(queue.ended().is_empty());
}

/// D8: another branch's unfinished request is not touched.
#[tokio::test]
async fn d8_another_branchs_requests_are_left_alone() {
    let db = TestDb::start().await;
    add(&db, MAIN, "KR-M", "WAITING", None).await;
    add(&db, B2, "KR-B2", "WAITING", None).await;
    let queue = Arc::new(FakeQueue::default());

    let ids = close_day(&db, queue.clone(), MAIN)
        .await
        .expect("close_day_in");

    assert_eq!(ids, vec!["KR-M".to_owned()]);
    assert_eq!(row(&db, B2, "KR-B2").await, "WAITING | - | -");
    assert_eq!(queue.ended(), vec![("KR-M".to_owned(), day_closed())]);
}
