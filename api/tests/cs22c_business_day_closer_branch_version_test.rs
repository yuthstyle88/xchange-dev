//! Up-front tests for the CS-22 follow-up: `BusinessDayCloser::sweep_at` raises `branch_versions` when it ends
//! something, so the views versioned by branch (`kiosk_queue`, `my_queue_ticket`) see the cancellations; the same
//! fix as the booking sweeper's (#1937). Review: PR 6274 issuecomment-6650397669 item 6, issuecomment-6653217971 §3.
//!
//! Row V1: defaults; a WAITING ticket issued 4 Nov 23:00 Bangkok; `sweep_at(5 Nov 06:01)` ends it -> MAIN's
//!         `branch_versions` rises by exactly 1
//! Row V2: the same sweep again ends nothing -> MAIN's `branch_versions` is unchanged (no per-minute churn)
//! Row V3: a branch with nothing to end keeps its version while another branch's sweep ends a ticket
//! Row V4: lock order. While another transaction holds MAIN's `branch_versions` row, as `BranchTx::run` does, the
//!         sweep waits for it before it locks any ticket: that transaction can still lock the WAITING ticket, and the
//!         sweep ends the ticket once it commits (review of PR 6297: the sweep took the ticket rows first)

mod common;
mod quote_common;

use std::sync::Arc;

use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use pos_infra::transactions::Transactor;
use quote_common::TestDb;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;
use xc_queue::infrastructure::repository::PgQueueRepository;
use xchange::application::workflows::business_day_close::BusinessDayCloser;

/// Bangkok wall-clock instant on 2026-11-`d`.
fn bkk(d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
    FixedOffset::east_opt(7 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 11, d, h, m, s)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

async fn open_queue(db: &TestDb, branch: &str) {
    sqlx::query(
        "INSERT INTO branch_service_status (branch_id, organization_id, queue_open) \
         SELECT id, organization_id, true FROM branches WHERE id = $1 \
         ON CONFLICT (branch_id) DO UPDATE SET queue_open = true",
    )
    .bind(branch)
    .execute(&db.pool)
    .await
    .expect("open_queue");
}

async fn issue(db: &TestDb, branch: &str, key: &str, now: DateTime<Utc>) -> QueueTicket {
    let issuer = TicketIssuer::new(
        BranchTx::new(Transactor::new(db.pool.clone())),
        Arc::new(PgQueueRepository::new(db.pool.clone())),
    );
    issuer
        .issue(&BranchId::new(branch), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

async fn ticket_status(db: &TestDb, id: &str) -> String {
    let (s,): (String,) = sqlx::query_as(
        "SELECT status || ' ' || COALESCE(ended_reason, '-') FROM queue_tickets WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("stored");
    s
}

async fn branch_version(db: &TestDb, branch: &str) -> i64 {
    BranchTx::new(Transactor::new(db.pool.clone()))
        .version(&BranchId::new(branch))
        .await
        .expect("branch version")
}

#[tokio::test]
async fn cs22c_v1_a_sweep_that_ends_a_ticket_raises_the_branch_version_once() {
    let db = TestDb::start().await;
    open_queue(&db, "MAIN").await;
    let ticket = issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;
    let before = branch_version(&db, "MAIN").await;

    let closer = BusinessDayCloser::new(db.pool.clone());
    let ended = closer.sweep_at(bkk(5, 6, 1, 0)).await.expect("sweep_at");

    assert!(ended >= 1, "expected at least 1 ticket ended, got {ended}");
    assert_eq!(ticket_status(&db, &ticket.id).await, "CANCELLED dayClosed");
    assert_eq!(
        branch_version(&db, "MAIN").await,
        before + 1,
        "V1: the sweep that ended work must raise MAIN's branch version exactly once"
    );

    db.finish().await;
}

#[tokio::test]
async fn cs22c_v2_a_sweep_that_ends_nothing_leaves_the_branch_version_alone() {
    let db = TestDb::start().await;
    open_queue(&db, "MAIN").await;
    issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;

    let closer = BusinessDayCloser::new(db.pool.clone());
    let sweep_time = bkk(5, 6, 1, 0);
    assert!(closer.sweep_at(sweep_time).await.expect("first sweep") >= 1);
    let before = branch_version(&db, "MAIN").await;

    let second = closer.sweep_at(sweep_time).await.expect("second sweep");

    assert_eq!(second, 0, "V2: the second sweep ends nothing");
    assert_eq!(
        branch_version(&db, "MAIN").await,
        before,
        "V2: an idle sweep must not raise the branch version"
    );

    db.finish().await;
}

#[tokio::test]
async fn cs22c_v3_a_branch_with_nothing_to_end_keeps_its_version() {
    let db = TestDb::start().await;
    sqlx::query(
        "INSERT INTO branches (id, name, timezone) VALUES ('B02', 'Branch B02', 'Asia/Bangkok')",
    )
    .execute(&db.pool)
    .await
    .expect("insert branch");
    open_queue(&db, "MAIN").await;
    open_queue(&db, "B02").await;
    issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;
    let main_before = branch_version(&db, "MAIN").await;
    let b02_before = branch_version(&db, "B02").await;

    let closer = BusinessDayCloser::new(db.pool.clone());
    assert!(closer.sweep_at(bkk(5, 6, 1, 0)).await.expect("sweep_at") >= 1);

    assert_eq!(branch_version(&db, "MAIN").await, main_before + 1);
    assert_eq!(
        branch_version(&db, "B02").await,
        b02_before,
        "V3: B02 had nothing to end, so its version must not move"
    );

    db.finish().await;
}

#[tokio::test]
async fn cs22c_v4_the_sweep_takes_the_branch_lock_before_the_ticket_rows() {
    let db = TestDb::start().await;
    open_queue(&db, "MAIN").await;
    let ticket = issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;

    // A command's transaction holds MAIN's branch lock, as `BranchTx::run` takes it first.
    let mut held = db.pool.begin().await.expect("begin held");
    sqlx::query("SELECT version FROM branch_versions WHERE branch_id = 'MAIN' FOR UPDATE")
        .execute(&mut *held)
        .await
        .expect("hold MAIN's branch lock");

    let pool = db.pool.clone();
    let sweep =
        tokio::spawn(async move { BusinessDayCloser::new(pool).sweep_at(bkk(5, 6, 1, 0)).await });
    // Give the sweep time to reach its first lock.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // The held transaction can still lock the ticket: the sweep is waiting on the branch lock, not holding the row.
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *held)
        .await
        .expect("lock_timeout");
    sqlx::query("SELECT id FROM queue_tickets WHERE id = $1::uuid FOR UPDATE")
        .bind(&ticket.id)
        .execute(&mut *held)
        .await
        .expect("V4: the sweep must not hold the ticket row while it waits for the branch lock");
    assert_eq!(
        ticket_status(&db, &ticket.id).await,
        "WAITING -",
        "V4: nothing is ended while the branch lock is held"
    );
    held.commit().await.expect("release the branch lock");

    let ended = sweep.await.expect("join").expect("sweep_at");
    assert!(
        ended >= 1,
        "V4: the sweep ends the ticket once the lock is free, got {ended}"
    );
    assert_eq!(ticket_status(&db, &ticket.id).await, "CANCELLED dayClosed");

    db.finish().await;
}
