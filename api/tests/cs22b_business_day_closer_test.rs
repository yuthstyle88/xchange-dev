//! Behaviour tests for ticket CS-22b (#6214): `BusinessDayCloser`.
//!
//! Row B1: defaults; a WAITING ticket issued 4 Nov 23:00 Bangkok; `sweep_at(5 Nov 06:01)` -> CANCELLED `dayClosed`; returns >= 1
//! Row B2: the same sweep again -> returns 0; `queue_version` unchanged
//! Row B3: a branch with no live template -> skipped without error; other branches still swept

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

async fn queue_version(db: &TestDb, branch: &str) -> i64 {
    let (v,): (i64,) = sqlx::query_as(
        "SELECT COALESCE((SELECT queue_version FROM branch_service_status WHERE branch_id = $1), 0)::bigint",
    )
    .bind(branch)
    .fetch_one(&db.pool)
    .await
    .expect("queue_version");
    v
}

#[tokio::test]
async fn cs22b_b1_defaults_waiting_ticket_cancelled_day_closed() {
    let db = TestDb::start().await;
    open_queue(&db, "MAIN").await;
    let ticket = issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;

    let closer = BusinessDayCloser::new(db.pool.clone());
    let sweep_time = bkk(5, 6, 1, 0); // 5 Nov 06:01 Bangkok
    let ended = closer.sweep_at(sweep_time).await.expect("sweep_at");

    assert!(ended >= 1, "expected at least 1 ticket ended, got {ended}");
    assert_eq!(
        ticket_status(&db, &ticket.id).await,
        "CANCELLED dayClosed",
        "B1: WAITING ticket issued before cutover must be CANCELLED dayClosed"
    );

    db.finish().await;
}

#[tokio::test]
async fn cs22b_b2_same_sweep_again_returns_zero_queue_version_unchanged() {
    let db = TestDb::start().await;
    open_queue(&db, "MAIN").await;
    let ticket = issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;

    let closer = BusinessDayCloser::new(db.pool.clone());
    let sweep_time = bkk(5, 6, 1, 0);
    let first = closer.sweep_at(sweep_time).await.expect("first sweep");
    assert!(first >= 1);
    assert_eq!(ticket_status(&db, &ticket.id).await, "CANCELLED dayClosed");

    let version_before = queue_version(&db, "MAIN").await;
    let second = closer.sweep_at(sweep_time).await.expect("second sweep");

    assert_eq!(second, 0, "B2: second sweep should return 0");
    let version_after = queue_version(&db, "MAIN").await;
    assert_eq!(
        version_after, version_before,
        "B2: queue_version must remain unchanged on no-op sweep"
    );

    db.finish().await;
}

#[tokio::test]
async fn cs22b_b3_branch_with_no_live_template_skipped_without_error() {
    let db = TestDb::start().await;
    sqlx::query("INSERT INTO branches (id, name, timezone) VALUES ('B02', 'Branch B02', 'Asia/Bangkok')")
        .execute(&db.pool)
        .await
        .expect("insert branch");
    sqlx::query("UPDATE shift_templates SET retired_at = now() WHERE branch_id = 'B02'")
        .execute(&db.pool)
        .await
        .expect("retire shift templates");

    open_queue(&db, "MAIN").await;
    let ticket = issue(&db, "MAIN", "w-4nov-2300", bkk(4, 23, 0, 0)).await;

    let closer = BusinessDayCloser::new(db.pool.clone());
    let ended = closer
        .sweep_at(bkk(5, 6, 1, 0))
        .await
        .expect("sweep_at should succeed without error");

    assert!(ended >= 1, "MAIN should still be swept, got {ended}");
    assert_eq!(ticket_status(&db, &ticket.id).await, "CANCELLED dayClosed");

    db.finish().await;
}
