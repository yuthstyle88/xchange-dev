//! S5 of the FE survey section B (#3408; design `docs/design/blueprint/section-b-gaps.md` §S5, PR 3387).
//!
//! 🔴 Written by the ticket writer claude/studio-claude-4 before the implementation. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! The `queue_called` notification links to the ticket it is about:
//! `NotificationObject { object_type: "QUEUE_TICKET", object_id: <ticket id> }`.

#![allow(dead_code)]

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

/// Writes each request into `s5_probe` through the transaction it is handed.
struct Probe;

impl Notifications for Probe {
    fn notify_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        request: &'a NotificationRequest,
    ) -> BoxFuture<'a, AppResult<NotificationId>> {
        Box::pin(async move {
            let (object_type, object_id) = match request.object() {
                Some(o) => (Some(o.object_type.clone()), Some(o.object_id.clone())),
                None => (None, None),
            };
            sqlx::query(
                "INSERT INTO s5_probe (template_key, object_type, object_id) VALUES ($1, $2, $3)",
            )
            .bind(request.template_key())
            .bind(object_type)
            .bind(object_id)
            .execute(&mut **tx)
            .await?;
            Ok(NotificationId(uuid::Uuid::new_v4().to_string()))
        })
    }
}

/// Every committed request, as `template | object_type | object_id` ("-" when absent).
async fn written(db: &TestDb) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT template_key || ' | ' || COALESCE(object_type, '-') || ' | ' || COALESCE(object_id, '-') \
         FROM s5_probe ORDER BY seq",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read s5_probe");
    rows.into_iter().map(|(r,)| r).collect()
}

/// An open MAIN queue, its services, and a desk wired to the probe.
async fn open_main() -> (TestDb, Queue, QueueDesk) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    db.exec(
        "CREATE TABLE s5_probe (seq bigserial PRIMARY KEY, template_key text NOT NULL, \
             object_type text NULL, object_id text NULL)",
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

#[tokio::test]
async fn s5_the_called_notification_names_the_queue_ticket_as_its_object() {
    let (db, q, desk) = open_main().await;
    let a = on_duty(&db, &q, "MAIN", "Teller A", at(2, 0)).await;
    let ticket = walk_in(&db, "w1", at(3, 0)).await;
    let (customer, _user) = add_customer(&db, true).await;
    link(&db, &ticket, &customer).await;

    desk.call(&t(&a), &main_branch(), &ticket.id, at(3, 5))
        .await
        .expect("A calls the ticket");

    assert_eq!(
        written(&db).await,
        vec![format!("queue_called | QUEUE_TICKET | {}", ticket.id)],
        "the notice links to the ticket it is about"
    );
    db.finish().await;
}
