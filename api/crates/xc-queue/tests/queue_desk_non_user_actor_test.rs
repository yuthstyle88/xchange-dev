//! RBAC §11 Q1 (#3810) — `QueueDesk` when the caller's actor id is not a UUID (`"owner"`), plus
//! the unchanged UUID teller cases of §9.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod queue_common;
mod queue_desk_common;

use chrono::{DateTime, TimeZone, Utc};
use queue_common::*;
use queue_desk_common::*;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_queue::domain::ticket::issue_source::IssueSource;
use xc_queue::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};

async fn walk_in(db: &TestDb, key: &str, now: DateTime<Utc>) -> QueueTicket {
    db.issuer()
        .issue(&main_branch(), IssueSource::Desk, key, now)
        .await
        .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
}

async fn open_main() -> (TestDb, Queue) {
    let db = TestDb::start().await;
    db.open_queue("MAIN").await;
    let q = queue(&db);
    (db, q)
}

fn owner() -> Actor {
    let since = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        "owner",
        ORG_1,
        vec![
            ScopedRole::new(Role::Executive, None, since, None),
            ScopedRole::new(Role::Admin, None, since, None),
        ],
        None,
    )
}

#[tokio::test]
async fn owner_calling_a_waiting_ticket_is_refused_no_open_shift_and_ticket_stays_waiting() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;

    assert_eq!(
        refused(
            q.desk
                .call(&owner(), &main_branch(), &w1.id, at(3, 10))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    let mut tx = db.pool.begin().await.expect("begin");
    assert_eq!(
        refused(
            q.desk
                .call_in(&mut tx, &owner(), &main_branch(), &w1.id, at(3, 11))
                .await
        ),
        "noOpenShift"
    );
    tx.commit().await.expect("commit");
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    db.finish().await;
}

#[tokio::test]
async fn owner_on_call_next_and_call_next_in_is_refused_no_open_shift() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;

    assert_eq!(
        refused(q.desk.call_next(&owner(), &main_branch(), at(3, 10)).await),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    let mut tx = db.pool.begin().await.expect("begin");
    assert_eq!(
        refused(
            q.desk
                .call_next_in(&mut tx, &owner(), &main_branch(), at(3, 11))
                .await
        ),
        "noOpenShift"
    );
    tx.commit().await.expect("commit");
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    db.finish().await;
}

#[tokio::test]
async fn uuid_teller_cases_remain_unchanged() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;
    let w2 = walk_in(&db, "w2", at(3, 1)).await;

    // Row 4: a uuid teller with no counter -> noOpenShift
    let no_counter = db.add_user("No Counter").await;
    assert_eq!(
        refused(
            q.desk
                .call(
                    &teller(&no_counter, "MAIN"),
                    &main_branch(),
                    &w1.id,
                    at(3, 5)
                )
                .await
        ),
        "noOpenShift"
    );

    // Row 3: a uuid teller with an OPEN counter calls a WAITING ticket -> CALLED
    let active = on_duty(&db, &q, "MAIN", "Active Teller", at(2, 0)).await;
    let called = q
        .desk
        .call(&teller(&active, "MAIN"), &main_branch(), &w1.id, at(3, 10))
        .await
        .expect("active teller calls waiting ticket");
    assert_eq!(called.status, TicketStatus::Called);
    assert_eq!(called.counter_teller_id.as_deref(), Some(active.as_str()));

    // Row 6: a uuid teller already serving -> alreadyServing
    assert_eq!(
        refused(
            q.desk
                .call(&teller(&active, "MAIN"), &main_branch(), &w2.id, at(3, 11))
                .await
        ),
        "alreadyServing"
    );

    // Row 5: a uuid teller with a PAUSED counter -> counterPaused
    let paused = on_duty(&db, &q, "MAIN", "Paused Teller", at(2, 0)).await;
    q.counters
        .pause(&teller(&paused, "MAIN"), &main_branch(), at(3, 12))
        .await
        .expect("pause counter");
    assert_eq!(
        refused(
            q.desk
                .call(&teller(&paused, "MAIN"), &main_branch(), &w2.id, at(3, 13))
                .await
        ),
        "counterPaused"
    );

    db.finish().await;
}

#[tokio::test]
async fn owner_recalling_a_waiting_ticket_is_refused_no_open_shift_and_ticket_stays_waiting() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;

    assert_eq!(
        refused(
            q.desk
                .recall(&owner(), &main_branch(), &w1.id, at(3, 10))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    let mut tx = db.pool.begin().await.expect("begin");
    assert_eq!(
        refused(
            q.desk
                .recall_in(&mut tx, &owner(), &main_branch(), &w1.id, at(3, 11))
                .await
        ),
        "noOpenShift"
    );
    tx.commit().await.expect("commit");
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    db.finish().await;
}

#[tokio::test]
async fn owner_skipping_a_waiting_ticket_is_refused_no_open_shift_and_ticket_stays_waiting() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;

    assert_eq!(
        refused(
            q.desk
                .skip(&owner(), &main_branch(), &w1.id, at(3, 10))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    let mut tx = db.pool.begin().await.expect("begin");
    assert_eq!(
        refused(
            q.desk
                .skip_in(&mut tx, &owner(), &main_branch(), &w1.id, at(3, 11))
                .await
        ),
        "noOpenShift"
    );
    tx.commit().await.expect("commit");
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    db.finish().await;
}

#[tokio::test]
async fn owner_starting_a_waiting_ticket_is_refused_no_open_shift_and_ticket_stays_waiting() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;

    assert_eq!(
        refused(
            q.desk
                .start(&owner(), &main_branch(), &w1.id, at(3, 10))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    let mut tx = db.pool.begin().await.expect("begin");
    assert_eq!(
        refused(
            q.desk
                .start_in(&mut tx, &owner(), &main_branch(), &w1.id, at(3, 11))
                .await
        ),
        "noOpenShift"
    );
    tx.commit().await.expect("commit");
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    db.finish().await;
}

#[tokio::test]
async fn owner_finishing_a_waiting_ticket_is_refused_no_open_shift_and_ticket_stays_waiting() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;

    assert_eq!(
        refused(
            q.desk
                .finish(&owner(), &main_branch(), &w1.id, at(3, 10))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    let mut tx = db.pool.begin().await.expect("begin");
    assert_eq!(
        refused(
            q.desk
                .finish_in(&mut tx, &owner(), &main_branch(), &w1.id, at(3, 11))
                .await
        ),
        "noOpenShift"
    );
    tx.commit().await.expect("commit");
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    db.finish().await;
}

#[tokio::test]
async fn owner_cancelling_a_waiting_ticket_is_refused_no_open_shift_and_ticket_stays_waiting() {
    let (db, q) = open_main().await;
    let w1 = walk_in(&db, "w1", at(3, 0)).await;

    assert_eq!(
        refused(
            q.desk
                .cancel(&owner(), &main_branch(), &w1.id, "customer left", at(3, 10))
                .await
        ),
        "noOpenShift"
    );
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    let mut tx = db.pool.begin().await.expect("begin");
    assert_eq!(
        refused(
            q.desk
                .cancel_in(
                    &mut tx,
                    &owner(),
                    &main_branch(),
                    &w1.id,
                    "customer left",
                    at(3, 11),
                )
                .await
        ),
        "noOpenShift"
    );
    tx.commit().await.expect("commit");
    assert_eq!(stored(&db, &w1.id).await, "WAITING W001");

    db.finish().await;
}
