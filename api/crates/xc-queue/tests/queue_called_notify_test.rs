//! APP-10 (#1001, design `docs/design/blueprint/customer-app.md` P8, §5 `QueueDesk --> Notifications`,
//! §9.1 row APP-10): calling a ticket tells its customer. Written before the implementation by the
//! ticket author (claude/89bd61d9). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Signatures this test assumes (`[required]` once pushed):
//! - `QueueDesk::with_notifications(self, Arc<dyn xc_kernel::notifications::Notifications>) -> Self`.
//!   `QueueDesk::new(BranchTx, Arc<dyn QueueRepository>, StatusService)` keeps QU-A2's signature,
//!   so QU-A2's harness and tests stay unedited; a desk built without the port writes nothing.
//! - Each call writes one `NotificationRequest` through `notify_in(tx, …)` in the call's own branch
//!   transaction: recipient = `customers.user_id` of the ticket's `customer_id`, template key
//!   `queue_called`, event id `queue.called:<ticket id>:<n>` where `<n>` counts this ticket's calls
//!   and recalls so far including this one (call → 1, first recall → 2, …), params exactly
//!   `{displayNumber: <ticket's display_number>, branchName: <branches.name>}` (no name, amount
//!   or document number, P10).
//!
//! The port here is a probe that writes its row through the transaction it is handed, so a row is
//! counted only if the call's transaction commits: "the call rolls back → none" is what the probe
//! table shows, whatever order the implementation writes in.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use queue_common::*;
use queue_desk_common::*;
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::Actor;
use xc_kernel::notifications::{NotificationId, NotificationRequest, Notifications};
use xc_queue::application::desk::queue_desk::QueueDesk;
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::QueueTicket;

/// Writes each request into `app10_probe` through the transaction it is handed.
struct Probe;

impl Notifications for Probe {
    fn notify_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        request: &'a NotificationRequest,
    ) -> BoxFuture<'a, AppResult<NotificationId>> {
        Box::pin(async move {
            let params = request
                .params()
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(";");
            sqlx::query(
                "INSERT INTO app10_probe (recipient, event_id, template_key, params) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(request.recipient_user_id())
            .bind(request.event_id())
            .bind(request.template_key())
            .bind(params)
            .execute(&mut **tx)
            .await?;
            Ok(NotificationId(uuid::Uuid::new_v4().to_string()))
        })
    }
}

/// One committed request, as `recipient | event | template | params`.
async fn written(db: &TestDb) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT recipient || ' | ' || event_id || ' | ' || template_key || ' | ' || params \
         FROM app10_probe ORDER BY seq",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read app10_probe");
    rows.into_iter().map(|(r,)| r).collect()
}

/// An open MAIN queue, its services, and a desk wired to the probe.
async fn open_main() -> (TestDb, Queue, QueueDesk) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.exec(
        "CREATE TABLE app10_probe (seq bigserial PRIMARY KEY, recipient text NOT NULL, \
             event_id text NOT NULL, template_key text NOT NULL, params text NOT NULL)",
    )
    .await;
    let q = queue(&db);
    let desk = q.desk.clone().with_notifications(Arc::new(Probe));
    (db, q, desk)
}

fn t(user_id: &str) -> Actor {
    teller(user_id, "MAIN")
}

async fn walk_in(db: &TestDb, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&main_branch(), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

/// A customer of organisation 1; with `has_user`, linked to a new user. Answers (customer id,
/// user id or "").
async fn add_customer(db: &TestDb, has_user: bool) -> (String, String) {
    let customer = unique("CUST");
    let user = if has_user {
        db.add_user(&unique("cust-user")).await
    } else {
        String::new()
    };
    let user_sql = if has_user {
        format!("'{user}'::uuid")
    } else {
        "NULL".to_owned()
    };
    db.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, user_id) \
         VALUES ('{customer}', '{ORG_1}', '{customer}', 'Somsri Jaidee', 'LIVE', now(), {user_sql})"
    ))
    .await;
    (customer, user)
}

async fn link(db: &TestDb, ticket: &QueueTicket, customer: &str) {
    db.exec(&format!(
        "UPDATE queue_tickets SET customer_id = '{customer}' WHERE id = '{}'::uuid",
        ticket.id
    ))
    .await;
}

async fn main_name(db: &TestDb) -> String {
    db.text("SELECT name FROM branches WHERE id = 'MAIN'")
        .await
        .expect("MAIN has a name")
}

fn expected(user: &str, ticket: &QueueTicket, n: u32, branch_name: &str) -> String {
    format!(
        "{user} | queue.called:{}:{n} | queue_called | branch=MAIN;branchName={branch_name};displayNumber={}",
        ticket.id, ticket.display_number
    )
}

#[tokio::test]
async fn calling_a_ticket_whose_customer_has_a_user_writes_one_called_notification() {
    let (db, q, desk) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ticket = walk_in(&db, "w1", at(3, 0)).await;
    let (customer, user) = add_customer(&db, true).await;
    link(&db, &ticket, &customer).await;

    desk.call(&t(&a), &main_branch(), &ticket.id, at(3, 5))
        .await
        .expect("A calls the ticket");

    assert_eq!(
        written(&db).await,
        vec![expected(&user, &ticket, 1, &main_name(&db).await)]
    );
    db.finish().await;
}

#[tokio::test]
async fn call_next_tells_the_customer_of_the_ticket_it_calls() {
    let (db, q, desk) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ticket = walk_in(&db, "w1", at(3, 0)).await;
    let (customer, user) = add_customer(&db, true).await;
    link(&db, &ticket, &customer).await;

    let called = desk
        .call_next(&t(&a), &main_branch(), at(3, 5))
        .await
        .expect("A calls next");
    assert_eq!(called.id, ticket.id);

    assert_eq!(
        written(&db).await,
        vec![expected(&user, &ticket, 1, &main_name(&db).await)]
    );
    db.finish().await;
}

#[tokio::test]
async fn each_recall_writes_one_more_notification_with_the_next_call_number() {
    let (db, q, desk) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ticket = walk_in(&db, "w1", at(3, 0)).await;
    let (customer, user) = add_customer(&db, true).await;
    link(&db, &ticket, &customer).await;
    let name = main_name(&db).await;

    desk.call(&t(&a), &main_branch(), &ticket.id, at(3, 5))
        .await
        .expect("A calls");
    desk.recall(&t(&a), &main_branch(), &ticket.id, at(3, 7))
        .await
        .expect("A recalls once");
    desk.recall(&t(&a), &main_branch(), &ticket.id, at(3, 9))
        .await
        .expect("A recalls twice");

    assert_eq!(
        written(&db).await,
        vec![
            expected(&user, &ticket, 1, &name),
            expected(&user, &ticket, 2, &name),
            expected(&user, &ticket, 3, &name),
        ]
    );
    db.finish().await;
}

#[tokio::test]
async fn a_ticket_with_no_customer_writes_none_on_call_or_recall() {
    let (db, q, desk) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ticket = walk_in(&db, "w1", at(3, 0)).await;

    desk.call(&t(&a), &main_branch(), &ticket.id, at(3, 5))
        .await
        .expect("A calls a walk-in with no customer");
    desk.recall(&t(&a), &main_branch(), &ticket.id, at(3, 7))
        .await
        .expect("A recalls it");

    assert_eq!(written(&db).await, Vec::<String>::new());
    assert_eq!(
        stored(&db, &ticket.id).await,
        format!("CALLED {}", ticket.display_number)
    );
    db.finish().await;
}

#[tokio::test]
async fn a_customer_with_no_user_gets_none_and_the_call_still_happens() {
    let (db, q, desk) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ticket = walk_in(&db, "w1", at(3, 0)).await;
    let (customer, _) = add_customer(&db, false).await;
    link(&db, &ticket, &customer).await;

    desk.call(&t(&a), &main_branch(), &ticket.id, at(3, 5))
        .await
        .expect("A calls; the customer has no app user");

    assert_eq!(written(&db).await, Vec::<String>::new());
    assert_eq!(
        stored(&db, &ticket.id).await,
        format!("CALLED {}", ticket.display_number)
    );
    db.finish().await;
}

#[tokio::test]
async fn a_refused_call_or_recall_writes_none() {
    let (db, q, desk) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let b = on_duty(&db, &q, "MAIN", "Teller B", at(2, 0)).await;
    let ticket = walk_in(&db, "w1", at(3, 0)).await;
    let (customer, user) = add_customer(&db, true).await;
    link(&db, &ticket, &customer).await;

    desk.call(&t(&a), &main_branch(), &ticket.id, at(3, 5))
        .await
        .expect("A calls");
    // B calls the same ticket: it is no longer WAITING.
    assert_eq!(
        refused(
            desk.call(&t(&b), &main_branch(), &ticket.id, at(3, 6))
                .await
        ),
        "staleVersion"
    );
    // B recalls A's ticket: not B's (QU-3).
    assert_eq!(
        refused(
            desk.recall(&t(&b), &main_branch(), &ticket.id, at(3, 7))
                .await
        ),
        "notYourTicket"
    );

    assert_eq!(
        written(&db).await,
        vec![expected(&user, &ticket, 1, &main_name(&db).await)],
        "only A's call is told; the refused call and recall roll back with nothing written"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_desk_built_without_the_port_calls_and_writes_nothing() {
    let (db, q, _) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ticket = walk_in(&db, "w1", at(3, 0)).await;
    let (customer, _) = add_customer(&db, true).await;
    link(&db, &ticket, &customer).await;

    q.desk
        .call(&t(&a), &main_branch(), &ticket.id, at(3, 5))
        .await
        .expect("the QU-A2 desk calls as before");

    assert_eq!(written(&db).await, Vec::<String>::new());
    db.finish().await;
}
