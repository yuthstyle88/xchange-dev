//! QU-A1 (#556 design row QU-2) — `TicketIssuer` and the `QueueTickets` port against a real
//! PostgreSQL: daily numbers per branch and line, the branch's local date, concurrent issuance,
//! idempotent retries, one ticket per source, the closed queue, and atomicity with the caller.
//! Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/queue-tickets.md` Q2–Q4, Q11, Q12, §4.3 rows 1–3, §9.1 row
//! QU-2. Owner Q-A1 (WALK_IN and BOOKING count apart).
//!
//! Signatures this test assumes (QU-A1 scaffolding, `[required]` in the ticket):
//! - `TicketIssuer::new(BranchTx, Arc<dyn QueueRepository>)`, `TicketIssuer::issue(&BranchId,
//!   IssueSource, key, now) -> AppResult<QueueTicket>`.
//! - `TicketIssuer: QueueTickets` (`xc_kernel::queue_tickets`), usable as `Arc<dyn QueueTickets>`.
//! - A closed queue answers `AppError::Validation("queueClosed")`.
//! - A key already stored for a different source answers `AppError::Conflict("idempotencyConflict")`
//!   (the lane controller's ruling 4 on #879, the CM-2/CM-3 convention).
//!
//! The last two tests come from the lane controller's rulings 4 and 5 on #879:
//! `a_key_reused_for_a_different_source_is_refused_and_takes_no_number` and
//! `a_branch_created_after_m1_has_no_status_row_and_is_closed`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pos_error::AppError;
use queue_common::*;
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::{QueueTicketSource, QueueTickets};
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};
use xc_queue::domain::ticket::service_type::ServiceType;

fn main_branch() -> BranchId {
    BranchId::new("MAIN")
}

/// 2026-11-02 10:00 UTC = 17:00 in Bangkok.
fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, h, m, 0).unwrap()
}

fn date(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

fn kiosk(id: &str) -> IssueSource {
    IssueSource::Kiosk {
        request_id: id.to_owned(),
    }
}

fn booking(id: &str) -> IssueSource {
    IssueSource::Booking {
        booking_id: id.to_owned(),
    }
}

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
        .unwrap_or_else(|e| panic!("issue {key} on {branch}: {e:?}"))
}

fn is_queue_closed(err: &AppError) -> bool {
    matches!(err, AppError::Validation(code) if code == "queueClosed")
}

#[tokio::test]
async fn numbers_start_at_one_per_branch_day_and_line_and_the_two_lines_count_apart() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.add_branch("B2", "Asia/Bangkok").await;
    db.open_queue("B2").await;
    db.add_booking("MAIN", "BK-1").await;

    let w1 = issue(&db, "MAIN", IssueSource::Desk, "w1", at(3, 0)).await;
    let w2 = issue(&db, "MAIN", IssueSource::Desk, "w2", at(3, 1)).await;
    let b1 = issue(&db, "MAIN", booking("BK-1"), "b1", at(3, 2)).await;
    let w3 = issue(&db, "MAIN", IssueSource::Desk, "w3", at(3, 3)).await;
    let other = issue(&db, "B2", IssueSource::Desk, "o1", at(3, 4)).await;

    let seen = |t: &QueueTicket| (t.service_type, t.sequence, t.display_number.clone());
    assert_eq!(seen(&w1), (ServiceType::WalkIn, 1, "W001".to_owned()));
    assert_eq!(seen(&w2), (ServiceType::WalkIn, 2, "W002".to_owned()));
    assert_eq!(
        seen(&b1),
        (ServiceType::Booking, 1, "B001".to_owned()),
        "the booking line has its own sequence (owner Q-A1)"
    );
    assert_eq!(
        seen(&w3),
        (ServiceType::WalkIn, 3, "W003".to_owned()),
        "a booking ticket takes no walk-in number"
    );
    assert_eq!(
        seen(&other),
        (ServiceType::WalkIn, 1, "W001".to_owned()),
        "another branch starts at 1"
    );
    for t in [&w1, &w2, &b1, &w3] {
        assert_eq!(t.status, TicketStatus::Waiting);
        assert_eq!(t.branch_id, main_branch());
        assert_eq!(t.organization_id, ORG_1);
        assert_eq!(t.service_date, date(2), "10:00 Bangkok on 2 November");
    }
    assert_eq!(b1.booking_id.as_deref(), Some("BK-1"));
    db.finish().await;
}

#[tokio::test]
async fn the_service_date_is_the_branchs_own_local_date() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await; // Asia/Bangkok, UTC+7
    db.add_branch("TYO", "Asia/Tokyo").await; // UTC+9
    db.open_queue("TYO").await;

    let late = issue(&db, "MAIN", IssueSource::Desk, "m1", at(16, 59)).await;
    let early = issue(&db, "MAIN", IssueSource::Desk, "m2", at(17, 1)).await;
    assert_eq!(
        (late.service_date, late.sequence),
        (date(2), 1),
        "23:59 in Bangkok is still 2 November"
    );
    assert_eq!(
        (early.service_date, early.sequence),
        (date(3), 1),
        "00:01 in Bangkok is 3 November, and the number starts again at 1"
    );

    let tyo_late = issue(&db, "TYO", IssueSource::Desk, "t1", at(14, 59)).await;
    let tyo_early = issue(&db, "TYO", IssueSource::Desk, "t2", at(15, 1)).await;
    let tyo_next = issue(&db, "TYO", IssueSource::Desk, "t3", at(15, 2)).await;
    assert_eq!(
        (tyo_late.service_date, tyo_late.sequence),
        (date(2), 1),
        "23:59 in Tokyo: the branch's timezone is read, not Bangkok's"
    );
    assert_eq!((tyo_early.service_date, tyo_early.sequence), (date(3), 1));
    assert_eq!((tyo_next.service_date, tyo_next.sequence), (date(3), 2));
    db.finish().await;
}

#[tokio::test]
async fn fifty_concurrent_issues_on_one_branch_give_one_to_fifty() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let issuer = db.issuer();
    let mut tasks = Vec::new();
    for n in 0..50 {
        let issuer = issuer.clone();
        tasks.push(tokio::spawn(async move {
            issuer
                .issue(
                    &BranchId::new("MAIN"),
                    IssueSource::Desk,
                    &format!("c{n}"),
                    at(3, 0),
                )
                .await
        }));
    }
    let mut sequences = Vec::new();
    for task in tasks {
        let ticket = task
            .await
            .expect("the task ran")
            .unwrap_or_else(|e| panic!("a concurrent issue failed: {e:?}"));
        sequences.push(ticket.sequence);
    }
    sequences.sort_unstable();
    assert_eq!(
        sequences,
        (1..=50).collect::<Vec<i32>>(),
        "no gap, no duplicate (§18 concurrent ticket issuance)"
    );
    assert_eq!(db.count("SELECT count(*) FROM queue_tickets").await, 50);
    assert_eq!(
        db.count("SELECT count(DISTINCT display_number) FROM queue_tickets")
            .await,
        50
    );
    assert_eq!(
        db.count("SELECT last_sequence::bigint FROM queue_sequences WHERE branch_id = 'MAIN'")
            .await,
        50
    );
    db.finish().await;
}

#[tokio::test]
async fn a_retry_with_the_same_key_returns_the_same_ticket_and_takes_no_number() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let first = issue(&db, "MAIN", IssueSource::Desk, "same", at(3, 0)).await;
    let again = issue(&db, "MAIN", IssueSource::Desk, "same", at(3, 5)).await;
    assert_eq!(
        again, first,
        "the retry answers the stored ticket unchanged"
    );
    assert_eq!(db.count("SELECT count(*) FROM queue_tickets").await, 1);
    assert_eq!(
        db.count("SELECT count(*) FROM queue_events").await,
        1,
        "the retry writes no second event"
    );
    let next = issue(&db, "MAIN", IssueSource::Desk, "other", at(3, 6)).await;
    assert_eq!(next.sequence, 2, "the retry consumed no number");
    db.finish().await;
}

#[tokio::test]
async fn a_kiosk_request_gets_one_ticket_whatever_the_key() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.add_kiosk_request("MAIN", "KR-1").await;
    let first = issue(&db, "MAIN", kiosk("KR-1"), "a", at(3, 0)).await;
    let second = issue(&db, "MAIN", kiosk("KR-1"), "b", at(3, 1)).await;
    assert_eq!(second, first, "one ticket per kiosk request (Q12)");
    assert_eq!(first.kiosk_request_id.as_deref(), Some("KR-1"));
    assert_eq!(first.service_type, ServiceType::WalkIn);
    assert_eq!(db.count("SELECT count(*) FROM queue_tickets").await, 1);
    db.finish().await;
}

#[tokio::test]
async fn a_booking_gets_one_active_ticket_whatever_the_key() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.add_booking("MAIN", "BK-1").await;
    let first = issue(&db, "MAIN", booking("BK-1"), "a", at(3, 0)).await;
    let second = issue(&db, "MAIN", booking("BK-1"), "b", at(3, 1)).await;
    assert_eq!(
        second, first,
        "one active ticket per booking (§16 relations)"
    );
    assert_eq!(first.service_type, ServiceType::Booking);
    assert_eq!(first.display_number, "B001");
    db.finish().await;
}

#[tokio::test]
async fn a_closed_queue_refuses_and_takes_no_number() {
    let db = TestDb::start().await;
    // M1 leaves MAIN's queue closed.
    let err = db
        .issuer()
        .issue(&main_branch(), IssueSource::Desk, "k1", at(3, 0))
        .await
        .expect_err("a closed queue issues nothing");
    assert!(is_queue_closed(&err), "queueClosed, got {err:?}");

    db.add_branch("NEW", "Asia/Bangkok").await; // created after M1: no status row
    let err = db
        .issuer()
        .issue(&BranchId::new("NEW"), IssueSource::Desk, "k2", at(3, 0))
        .await
        .expect_err("a branch without a status row is closed");
    assert!(is_queue_closed(&err), "queueClosed, got {err:?}");

    assert_eq!(db.count("SELECT count(*) FROM queue_tickets").await, 0);
    assert_eq!(
        db.count("SELECT count(*) FROM queue_sequences").await,
        0,
        "a refused issue takes no number"
    );
    db.open_queue("MAIN").await;
    let t = issue(&db, "MAIN", IssueSource::Desk, "k3", at(3, 1)).await;
    assert_eq!(t.sequence, 1, "no gap after the refusals");
    db.finish().await;
}

#[tokio::test]
async fn a_retry_after_the_queue_closed_still_returns_the_stored_ticket() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let first = issue(&db, "MAIN", IssueSource::Desk, "k1", at(3, 0)).await;
    db.close_queue("MAIN").await;
    let again = issue(&db, "MAIN", IssueSource::Desk, "k1", at(3, 1)).await;
    assert_eq!(
        again, first,
        "a retry of an applied issue answers as before"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_issue_stores_a_waiting_ticket_and_one_issue_event() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let t = issue(&db, "MAIN", IssueSource::Desk, "k1", at(3, 0)).await;
    assert_eq!(t.issued_at, at(3, 0));
    assert_eq!(
        (
            t.called_at,
            t.served_at,
            t.completed_at,
            t.counter_teller_id.clone()
        ),
        (None, None, None, None)
    );
    let stored = db
        .text(&format!(
            "SELECT status || ' ' || display_number || ' ' || service_type || ' ' || \
                 service_date::text || ' ' || idempotency_key || ' ' || organization_id::text \
             FROM queue_tickets WHERE id = '{}'::uuid",
            t.id
        ))
        .await;
    assert_eq!(
        stored.as_deref(),
        Some(format!("WAITING W001 WALK_IN 2026-11-02 k1 {ORG_1}").as_str()),
        "the row as §4.1 names its columns"
    );
    let event = db
        .text(&format!(
            "SELECT kind || ' ' || status FROM queue_events WHERE ticket_id = '{}'::uuid",
            t.id
        ))
        .await;
    assert_eq!(event.as_deref(), Some("ISSUE WAITING"));
    db.finish().await;
}

#[tokio::test]
async fn the_port_issues_inside_the_callers_transaction_and_rolls_back_with_it() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.add_kiosk_request("MAIN", "KR-1").await;
    let port: Arc<dyn QueueTickets> = Arc::new(db.issuer());

    let failing = port.clone();
    let rolled_back = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                let source = QueueTicketSource::Kiosk {
                    request_id: "KR-1".to_owned(),
                };
                failing
                    .issue_in(tx, &BranchId::new("MAIN"), &source, "kiosk:KR-1", at(3, 0))
                    .await?;
                Err::<(), _>(AppError::Validation(
                    "the caller fails after issuing".to_owned(),
                ))
            })
        })
        .await;
    assert!(rolled_back.is_err());
    assert_eq!(
        db.count("SELECT count(*) FROM queue_tickets").await,
        0,
        "the ticket rolls back with the caller (§09 atomicity)"
    );
    assert_eq!(db.count("SELECT count(*) FROM queue_sequences").await, 0);

    let issued = db
        .branch_tx()
        .run(&main_branch(), move |tx| {
            Box::pin(async move {
                let source = QueueTicketSource::Kiosk {
                    request_id: "KR-1".to_owned(),
                };
                port.issue_in(tx, &BranchId::new("MAIN"), &source, "kiosk:KR-1", at(3, 1))
                    .await
            })
        })
        .await
        .expect("the port issues inside the caller's transaction");
    assert_eq!(
        (
            issued.sequence,
            issued.display_number.as_str(),
            issued.service_type.as_str(),
            issued.service_date
        ),
        (1, "W001", "WALK_IN", date(2)),
        "the rolled-back attempt took no number"
    );
    assert_eq!(
        db.text(&format!(
            "SELECT kiosk_request_id FROM queue_tickets WHERE id = '{}'::uuid",
            issued.id
        ))
        .await
        .as_deref(),
        Some("KR-1")
    );
    db.finish().await;
}

fn is_idempotency_conflict(err: &AppError) -> bool {
    matches!(err, AppError::Conflict(code) if code == "idempotencyConflict")
}

/// Ruling 4 on #879: a key names one request. Sent again for another source, it is refused; it
/// never answers the other source's ticket and never issues a second one.
#[tokio::test]
async fn a_key_reused_for_a_different_source_is_refused_and_takes_no_number() {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.add_kiosk_request("MAIN", "KR-1").await;
    db.add_kiosk_request("MAIN", "KR-2").await;
    db.add_booking("MAIN", "BK-1").await;
    let first = issue(&db, "MAIN", kiosk("KR-1"), "k", at(3, 0)).await;
    assert_eq!(first.display_number, "W001");

    for (other, what) in [
        (kiosk("KR-2"), "another kiosk request"),
        (IssueSource::Desk, "the desk"),
        (booking("BK-1"), "a booking"),
    ] {
        let err = db
            .issuer()
            .issue(&main_branch(), other, "k", at(3, 1))
            .await
            .expect_err("a key already used for KR-1 issues nothing for another source");
        assert!(
            is_idempotency_conflict(&err),
            "{what} with KR-1's key: idempotencyConflict, got {err:?}"
        );
    }

    assert_eq!(
        db.count("SELECT count(*) FROM queue_tickets").await,
        1,
        "no second ticket is stored"
    );
    assert_eq!(db.count("SELECT count(*) FROM queue_events").await, 1);
    assert_eq!(
        db.count("SELECT count(*) FROM queue_sequences").await,
        1,
        "the booking line took no number"
    );
    assert_eq!(
        db.count("SELECT last_sequence::bigint FROM queue_sequences WHERE branch_id = 'MAIN'")
            .await,
        1,
        "the walk-in line took no number"
    );

    let again = issue(&db, "MAIN", kiosk("KR-1"), "k", at(3, 2)).await;
    assert_eq!(
        again, first,
        "the same key for the same source still answers"
    );
    let next = issue(&db, "MAIN", kiosk("KR-2"), "k2", at(3, 3)).await;
    assert_eq!(next.sequence, 2, "no gap after the refusals");
    db.finish().await;
}

/// Ruling 5 on #879: M1 gives a status row only to the branches that exist when it runs. A
/// branch created later has none, and that reads as closed (fails closed); QU-A2's open upserts
/// the row. The refusal is `queueClosed`, never an internal error, and it writes nothing.
#[tokio::test]
async fn a_branch_created_after_m1_has_no_status_row_and_is_closed() {
    let db = TestDb::start().await;
    db.add_branch("LATE", "Asia/Bangkok").await;
    db.add_kiosk_request("LATE", "KR-L").await;
    let status_rows = "SELECT count(*) FROM branch_service_status WHERE branch_id = 'LATE'";
    assert_eq!(db.count(status_rows).await, 0, "M1 ran before LATE existed");

    for (source, key) in [(IssueSource::Desk, "d1"), (kiosk("KR-L"), "kiosk:KR-L")] {
        let err = db
            .issuer()
            .issue(&BranchId::new("LATE"), source, key, at(3, 0))
            .await
            .expect_err("a branch without a status row issues nothing");
        assert!(is_queue_closed(&err), "{key}: queueClosed, got {err:?}");
    }

    assert_eq!(db.count("SELECT count(*) FROM queue_tickets").await, 0);
    assert_eq!(db.count("SELECT count(*) FROM queue_events").await, 0);
    assert_eq!(
        db.count("SELECT count(*) FROM queue_sequences").await,
        0,
        "a refused issue takes no number"
    );
    assert_eq!(
        db.count(status_rows).await,
        0,
        "issuing never creates the status row; only opening the queue does (QU-A2)"
    );

    db.open_queue("LATE").await; // the upsert QU-A2's open performs
    let t = issue(&db, "LATE", kiosk("KR-L"), "kiosk:KR-L", at(3, 1)).await;
    assert_eq!(
        (t.sequence, t.display_number.as_str()),
        (1, "W001"),
        "once open, the branch starts at 1"
    );
    db.finish().await;
}
