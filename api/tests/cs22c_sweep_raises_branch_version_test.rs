//! Up-front tests for CS-22c (re-review of PR 6274, out-of-scope item 1, ruled a real bug by the lane controller
//! claude/022a7453): when the business-day sweep ends waiting work, it raises the branch's `branch_versions` row, so a
//! client polling a VERSIONED view (`kiosk_queue`, `my_queue_ticket`) with `since` refetches and drops the ticket.
//! A sweep that ends nothing leaves the version alone, so pollers are not woken every minute. Written by the ticket
//! author claude/35271802 — the implementer must not edit, delete or weaken these.
//!
//! Harness from `cs22b_business_day_closer_test.rs`. MAIN keeps the defaults (cutover 06:00 Bangkok). Needs
//! `TEST_DATABASE_URL`.

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

/// The branch's `branch_versions.version` (0 when the row does not exist yet).
async fn branch_version(db: &TestDb, branch: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE((SELECT version FROM branch_versions WHERE branch_id = $1), 0)::bigint",
    )
    .bind(branch)
    .fetch_one(&db.pool)
    .await
    .expect("branch_version")
}

/// V1: the sweep at 5 Nov 06:01 ends a ticket issued 4 Nov 23:00 and raises MAIN's branch version.
#[tokio::test]
async fn v1_a_sweep_that_ends_work_raises_the_branch_version() {
    let db = TestDb::start().await;
    open_queue(&db, "MAIN").await;
    issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;
    let before = branch_version(&db, "MAIN").await;

    let ended = BusinessDayCloser::new(db.pool.clone())
        .sweep_at(bkk(5, 6, 1, 0))
        .await
        .expect("sweep_at");

    assert!(ended >= 1, "the ticket is ended: {ended}");
    assert!(
        branch_version(&db, "MAIN").await > before,
        "V1: a client polling with since = {before} must see a new version"
    );
    db.finish().await;
}

/// V2: the same sweep again ends nothing and leaves MAIN's branch version where it was.
#[tokio::test]
async fn v2_a_sweep_that_ends_nothing_leaves_the_version() {
    let db = TestDb::start().await;
    open_queue(&db, "MAIN").await;
    issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;
    let closer = BusinessDayCloser::new(db.pool.clone());
    assert!(
        closer.sweep_at(bkk(5, 6, 1, 0)).await.expect("first") >= 1,
        "the first sweep ends the ticket"
    );
    let before = branch_version(&db, "MAIN").await;

    let ended = closer.sweep_at(bkk(5, 6, 2, 0)).await.expect("second");

    assert_eq!(ended, 0, "nothing left to end");
    assert_eq!(
        branch_version(&db, "MAIN").await,
        before,
        "V2: an empty sweep must not wake every poller"
    );
    db.finish().await;
}
