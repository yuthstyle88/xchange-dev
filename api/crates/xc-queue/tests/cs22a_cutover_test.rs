//! Up-front tests for ticket CS-22a (docs/design/configurable-shifts.md §2.12, rows Q1, Q2, Q3, Q4b, Q4d, Q5b). Written
//! by the ticket author claude/35271802 — do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! The cutover ends the previous business day's waiting work; work a teller holds gets one more day. Seam:
//! `QueueDayHook::on_business_day_end(tx, branch, cutover, held_before, now) -> AppResult<usize>` (how many tickets
//! and kiosk requests it ended): WAITING tickets issued before `cutover` and CALLED tickets issued before
//! `held_before` (the previous cutover) become `CANCELLED dayClosed` with one CANCEL event each; plain SERVING tickets
//! are never ended; a run that ends nothing leaves `queue_version` alone. Kiosk rows (Q4, Q4b's kiosk half, Q5) are
//! not here.
//!
//! MAIN is Asia/Bangkok with a 06:00 cutover (the defaults and overlap24 agree). Instants are Bangkok wall-clock times.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use queue_common::*;
use queue_desk_common::*;
use xc_kernel::ids::BranchId;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;

/// Bangkok wall-clock instant on 2026-11-`d`.
fn bkk(d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
    FixedOffset::east_opt(7 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 11, d, h, m, s)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

async fn issue(db: &TestDb, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&main_branch(), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

/// `on_business_day_end` on MAIN in a branch transaction of its own.
async fn end_day(
    db: &TestDb,
    q: &Queue,
    cutover: DateTime<Utc>,
    held_before: DateTime<Utc>,
    now: DateTime<Utc>,
) -> usize {
    let day = q.day.clone();
    let b = main_branch();
    db.branch_tx()
        .run(&BranchId::new("MAIN"), move |tx| {
            Box::pin(async move {
                day.on_business_day_end(tx, &b, cutover, held_before, now)
                    .await
            })
        })
        .await
        .unwrap_or_else(|e| panic!("on_business_day_end on MAIN: {e:?}"))
}

/// `status reason` of a ticket as stored (`-` for no reason).
async fn ended(db: &TestDb, id: &str) -> String {
    db.text(&format!(
        "SELECT status || ' ' || COALESCE(ended_reason, '-') FROM queue_tickets WHERE id = '{id}'::uuid"
    ))
    .await
    .expect("stored")
}

async fn day_closed_events(db: &TestDb, id: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM queue_events WHERE ticket_id = '{id}'::uuid AND kind = 'CANCEL' \
         AND reason = 'dayClosed'"
    ))
    .await
}

async fn all_events(db: &TestDb) -> i64 {
    db.count("SELECT count(*) FROM queue_events").await
}

#[tokio::test]
async fn q1_q2_q5b_waiting_tickets_issued_before_the_cutover_end_and_from_it_on_stay() {
    let db = TestDb::start().await;
    let q = queue(&db);
    db.open_queue("MAIN").await;
    let last_night = issue(&db, "w-4nov-2300", bkk(4, 23, 0, 0)).await;
    let at_cutover = issue(&db, "w-5nov-0600", bkk(5, 6, 0, 0)).await;
    let after = issue(&db, "w-5nov-060030", bkk(5, 6, 0, 30)).await;

    let n = end_day(&db, &q, bkk(5, 6, 0, 0), bkk(4, 6, 0, 0), bkk(5, 6, 1, 0)).await;

    assert_eq!(
        ended(&db, &last_night.id).await,
        "CANCELLED dayClosed",
        "Q1: WAITING, issued 4 Nov 23:00"
    );
    assert_eq!(
        day_closed_events(&db, &last_night.id).await,
        1,
        "Q1: one CANCEL event with the reason"
    );
    assert_eq!(
        ended(&db, &after.id).await,
        "WAITING -",
        "Q2: issued 5 Nov 06:00:30"
    );
    assert_eq!(
        ended(&db, &at_cutover.id).await,
        "WAITING -",
        "Q5b: at the cutover is the new day"
    );
    assert_eq!(n, 1, "it answers how many it ended");
    db.finish().await;
}

#[tokio::test]
async fn q3_a_second_run_ends_nothing_and_leaves_the_queue_version_alone() {
    let db = TestDb::start().await;
    let q = queue(&db);
    db.open_queue("MAIN").await;
    let last_night = issue(&db, "w-4nov-2300", bkk(4, 23, 0, 0)).await;
    let before = queue_version(&db, "MAIN").await;

    let first = end_day(&db, &q, bkk(5, 6, 0, 0), bkk(4, 6, 0, 0), bkk(5, 6, 1, 0)).await;
    assert_eq!(first, 1);
    assert_eq!(ended(&db, &last_night.id).await, "CANCELLED dayClosed");
    let after_first = queue_version(&db, "MAIN").await;
    assert!(
        after_first > before,
        "a run that ended a ticket refreshes the status ({before} → {after_first})"
    );
    let events = all_events(&db).await;

    let second = end_day(&db, &q, bkk(5, 6, 0, 0), bkk(4, 6, 0, 0), bkk(5, 6, 2, 0)).await;
    assert_eq!(second, 0, "Q3: nothing left to end");
    assert_eq!(all_events(&db).await, events, "Q3: no new events");
    assert_eq!(
        queue_version(&db, "MAIN").await,
        after_first,
        "Q3: a run that ends nothing changes nothing (polled clients do not refetch)"
    );
    db.finish().await;
}

#[tokio::test]
async fn q4b_a_called_ticket_is_held_one_more_business_day() {
    let db = TestDb::start().await;
    let q = queue(&db);
    db.open_queue("MAIN").await;
    let teller_id = on_duty(&db, &q, "MAIN", "Teller C", bkk(4, 22, 0, 0)).await;
    let t = teller(&teller_id, "MAIN");
    let w = issue(&db, "c-5nov-0558", bkk(5, 5, 58, 0)).await;
    q.desk
        .call(&t, &main_branch(), &w.id, bkk(5, 5, 59, 0))
        .await
        .expect("the C teller calls the customer at 05:59");

    let n = end_day(&db, &q, bkk(5, 6, 0, 0), bkk(4, 6, 0, 0), bkk(5, 6, 1, 0)).await;
    assert_eq!(
        ended(&db, &w.id).await,
        "CALLED -",
        "Q4b: a teller holds it across the 5 Nov cutover"
    );
    assert_eq!(n, 0);

    // The next cutover: the ticket was issued before the previous one (5 Nov 06:00), so it is ended now.
    let n = end_day(&db, &q, bkk(6, 6, 0, 0), bkk(5, 6, 0, 0), bkk(6, 6, 1, 0)).await;
    assert_eq!(
        ended(&db, &w.id).await,
        "CANCELLED dayClosed",
        "Q4b: ended at the 6 Nov cutover if still open"
    );
    assert_eq!(day_closed_events(&db, &w.id).await, 1);
    assert_eq!(n, 1);
    db.finish().await;
}

#[tokio::test]
async fn q4d_a_plain_serving_ticket_is_never_ended_by_the_cutover() {
    let db = TestDb::start().await;
    let q = queue(&db);
    db.open_queue("MAIN").await;
    let teller_id = on_duty(&db, &q, "MAIN", "Teller C", bkk(3, 22, 0, 0)).await;
    let t = teller(&teller_id, "MAIN");
    let w = issue(&db, "s-3nov-2300", bkk(3, 23, 0, 0)).await;
    q.desk
        .call(&t, &main_branch(), &w.id, bkk(3, 23, 1, 0))
        .await
        .expect("call");
    q.desk
        .start(&t, &main_branch(), &w.id, bkk(3, 23, 2, 0))
        .await
        .expect("start serving");

    // Issued before both the 4 Nov and the 5 Nov cutover: still not touched.
    let n = end_day(&db, &q, bkk(5, 6, 0, 0), bkk(4, 6, 0, 0), bkk(5, 6, 1, 0)).await;
    assert_eq!(ended(&db, &w.id).await, "SERVING -", "Q4d");
    assert_eq!(n, 0);
    let n = end_day(&db, &q, bkk(6, 6, 0, 0), bkk(5, 6, 0, 0), bkk(6, 6, 1, 0)).await;
    assert_eq!(
        ended(&db, &w.id).await,
        "SERVING -",
        "Q4d: a teller finishes a ticket being served"
    );
    assert_eq!(n, 0);
    assert_eq!(
        db.count("SELECT count(*) FROM queue_events WHERE reason = 'dayClosed'")
            .await,
        0
    );
    db.finish().await;
}
