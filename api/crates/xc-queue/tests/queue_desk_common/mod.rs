//! QU-A2 — helpers shared by the desk, counter, status and day-hook tests, on top of QU-A1's
//! `queue_common` (which each test file also declares, and which this file does not change).
//! Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or
//! weaken; add helpers in a file of your own.
//!
//! Signatures this harness assumes (QU-A2 scaffolding, `[required]` once pushed):
//! - `StatusService::new(BranchTx, Arc<dyn QueueRepository>)`;
//! - `CounterService::new(BranchTx, Arc<dyn QueueRepository>, StatusService)` with
//!   `on_shift_opened(tx, &BranchId, teller_id, now)` / `on_shift_closed(tx, &BranchId, teller_id, now)`;
//! - `QueueDesk::new(BranchTx, Arc<dyn QueueRepository>, StatusService)`;
//! - `QueueDayHook::new(Arc<dyn QueueRepository>, StatusService)`.
//!
//! A teller's shift is opened the way CA-3 (#619, PR 871) opens it: an OPEN count, then a shift
//! with `teller_id` and `opened_by_count`, so `shifts_one_open_per_teller` applies and several
//! tellers of one branch each hold an OPEN shift.

#![allow(dead_code)]

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pos_error::AppError;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_queue::application::counter::counter_service::CounterService;
use xc_queue::application::day::queue_day_hook::QueueDayHook;
use xc_queue::application::desk::queue_desk::QueueDesk;
use xc_queue::application::status::status_service::StatusService;
use xc_queue::domain::repository::QueueRepository;
use xc_queue::infrastructure::repository::PgQueueRepository;

use super::queue_common::{ORG_1, TestDb};

/// 2026-11-02 at `h:m` UTC; MAIN is Asia/Bangkok, so 03:00 UTC is 10:00 on 2 November there.
pub fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, h, m, 0).unwrap()
}

/// 2026-11-`d`.
pub fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

pub fn main_branch() -> BranchId {
    BranchId::new("MAIN")
}

fn role(role: Role, branch: Option<&str>) -> ScopedRole {
    ScopedRole::new(
        role,
        branch.map(BranchId::new),
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        None,
    )
}

/// A TELLER of organisation 1 in `branch`.
pub fn teller(user_id: &str, branch: &str) -> Actor {
    Actor::new(user_id, ORG_1, vec![role(Role::Teller, Some(branch))], None)
}

/// A BRANCH_MANAGER of organisation 1 in `branch`.
pub fn manager(user_id: &str, branch: &str) -> Actor {
    Actor::new(
        user_id,
        ORG_1,
        vec![role(Role::BranchManager, Some(branch))],
        None,
    )
}

/// A customer of organisation 1 (Q10: no command moves a ticket).
pub fn customer(user_id: &str) -> Actor {
    Actor::new(
        user_id,
        ORG_1,
        vec![role(Role::Customer, None)],
        Some("CUST-1"),
    )
}

/// The queue's services, wired as QU-B will wire them.
#[derive(Clone)]
pub struct Queue {
    pub repo: Arc<dyn QueueRepository>,
    pub status: StatusService,
    pub counters: CounterService,
    pub desk: QueueDesk,
    pub day: QueueDayHook,
}

pub fn queue(db: &TestDb) -> Queue {
    let repo: Arc<dyn QueueRepository> = Arc::new(PgQueueRepository::new(db.pool.clone()));
    let status = StatusService::new(db.branch_tx(), repo.clone());
    Queue {
        counters: CounterService::new(db.branch_tx(), repo.clone(), status.clone()),
        desk: QueueDesk::new(db.branch_tx(), repo.clone(), status.clone()),
        day: QueueDayHook::new(repo.clone(), status.clone()),
        status,
        repo,
    }
}

/// The refusal code an error carries: `Validation(code)` and `Conflict(code)` give the code,
/// `Forbidden` gives `"forbidden"`.
pub fn code(err: &AppError) -> String {
    match err {
        AppError::Validation(c) | AppError::Conflict(c) => c.clone(),
        AppError::Forbidden => "forbidden".to_owned(),
        other => format!("unexpected {other:?}"),
    }
}

/// The code of a result that must be a refusal.
pub fn refused<T: std::fmt::Debug>(result: Result<T, AppError>) -> String {
    match result {
        Ok(v) => panic!("expected a refusal, got Ok({v:?})"),
        Err(e) => code(&e),
    }
}

/// Opens `teller_id`'s shift in `branch` the CA-3 way (an OPEN count, then the shift), and calls
/// `CounterService::on_shift_opened` in the same branch transaction. Answers the shift id.
pub async fn open_shift(
    db: &TestDb,
    q: &Queue,
    branch: &str,
    teller_id: &str,
    now: DateTime<Utc>,
) -> String {
    let shift_id = format!("SH-{}", super::queue_common::unique("t"));
    let count_id = db
        .text(&format!(
            "INSERT INTO cash_counts (organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
             VALUES ('{ORG_1}', '{branch}', 'OPEN', '{teller_id}'::uuid, now(), '{shift_id}') RETURNING id::text"
        ))
        .await
        .expect("an OPEN count");
    db.exec(&format!(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, \
             teller_id) \
         VALUES ('{shift_id}', '{branch}', 'OPEN', now(), '{teller_id}', '', '{count_id}', \
             '{teller_id}'::uuid)"
    ))
    .await;
    let counters = q.counters.clone();
    let b = BranchId::new(branch);
    let t = teller_id.to_owned();
    db.branch_tx()
        .run(&BranchId::new(branch), move |tx| {
            Box::pin(async move { counters.on_shift_opened(tx, &b, &t, now).await })
        })
        .await
        .unwrap_or_else(|e| panic!("on_shift_opened for {teller_id}: {e:?}"));
    shift_id
}

/// Closes the shift and calls `CounterService::on_shift_closed` in the same branch transaction.
pub async fn close_shift(
    db: &TestDb,
    q: &Queue,
    branch: &str,
    teller_id: &str,
    now: DateTime<Utc>,
) {
    db.exec(&format!(
        "UPDATE shifts SET status = 'CLOSED', closed_at = now(), closed_by = '{teller_id}' \
         WHERE branch_id = '{branch}' AND teller_id = '{teller_id}'::uuid AND status = 'OPEN'"
    ))
    .await;
    let counters = q.counters.clone();
    let b = BranchId::new(branch);
    let t = teller_id.to_owned();
    db.branch_tx()
        .run(&BranchId::new(branch), move |tx| {
            Box::pin(async move { counters.on_shift_closed(tx, &b, &t, now).await })
        })
        .await
        .unwrap_or_else(|e| panic!("on_shift_closed for {teller_id}: {e:?}"));
}

/// A user of organisation 1 with an OPEN shift and an ACTIVE counter in `branch`.
pub async fn on_duty(
    db: &TestDb,
    q: &Queue,
    branch: &str,
    name: &str,
    now: DateTime<Utc>,
) -> String {
    let id = db.add_user(name).await;
    open_shift(db, q, branch, &id, now).await;
    id
}

pub async fn active_counters(db: &TestDb, branch: &str) -> i64 {
    db.count(&format!(
        "SELECT active_counters::bigint FROM branch_service_status WHERE branch_id = '{branch}'"
    ))
    .await
}

pub async fn branch_version(db: &TestDb, branch: &str) -> i64 {
    db.count(&format!(
        "SELECT COALESCE((SELECT version FROM branch_versions WHERE branch_id = '{branch}'), 0)::bigint"
    ))
    .await
}

pub async fn queue_version(db: &TestDb, branch: &str) -> i64 {
    db.count(&format!(
        "SELECT queue_version FROM branch_service_status WHERE branch_id = '{branch}'"
    ))
    .await
}

/// `status display_number` of a ticket as stored.
pub async fn stored(db: &TestDb, ticket_id: &str) -> String {
    db.text(&format!(
        "SELECT status || ' ' || display_number FROM queue_tickets WHERE id = '{ticket_id}'::uuid"
    ))
    .await
    .unwrap_or_else(|| panic!("ticket {ticket_id} is stored"))
}

/// The ticket's events in order, as `KIND STATUS actor` (`-` when no person acted).
pub async fn events(db: &TestDb, ticket_id: &str) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT kind || ' ' || status || ' ' || COALESCE(actor_id::text, '-') FROM queue_events \
         WHERE ticket_id = '{ticket_id}'::uuid ORDER BY at, created_at"
    )))
    .fetch_all(&db.pool)
    .await
    .expect("read the events");
    rows.into_iter().map(|(r,)| r).collect()
}

/// A COMPLETED WALK_IN ticket of `branch` on `date`, served for `seconds`, finished at
/// `completed_at`. Sequences from 500 keep clear of the numbers the issuer takes.
pub async fn add_completed(
    db: &TestDb,
    branch: &str,
    date: NaiveDate,
    sequence: i32,
    seconds: i64,
    completed_at: DateTime<Utc>,
) {
    let served_at = completed_at - chrono::Duration::seconds(seconds);
    db.exec(&format!(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
             display_number, status, issued_at, called_at, served_at, completed_at, idempotency_key) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{branch}', '{date}', 'WALK_IN', {sequence}, 'W{sequence:03}', \
             'COMPLETED', '{served_at}', '{served_at}', '{served_at}', '{completed_at}', \
             'done-{branch}-{date}-{sequence}')"
    ))
    .await;
}

/// The stored estimate as `seconds low high` (`NULL NULL NULL` when unknown) and whether
/// `estimate_as_of` is set.
pub async fn estimate(db: &TestDb, branch: &str) -> (String, bool) {
    let row: (String, bool) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT COALESCE(estimate_seconds::text, 'NULL') || ' ' || \
                COALESCE(estimate_low_seconds::text, 'NULL') || ' ' || \
                COALESCE(estimate_high_seconds::text, 'NULL'), \
                estimate_as_of IS NOT NULL \
         FROM branch_service_status WHERE branch_id = '{branch}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("the branch has a status row");
    row
}

/// `StatusService::refresh_in` in a branch transaction of its own.
pub async fn refresh(db: &TestDb, q: &Queue, branch: &str, now: DateTime<Utc>) {
    let status = q.status.clone();
    let b = BranchId::new(branch);
    db.branch_tx()
        .run(&BranchId::new(branch), move |tx| {
            Box::pin(async move { status.refresh_in(tx, &b, now).await })
        })
        .await
        .unwrap_or_else(|e| panic!("refresh_in on {branch}: {e:?}"));
}
