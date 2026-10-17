//! Ticket #2702: concurrent notificationRead calls under different branch locks.
//! Behaviour table rows M1–M5 (§8).

mod common;

use std::time::Duration;

use common::*;

async fn read_at(db: &TestDb, id: &NotificationId) -> Option<chrono::DateTime<chrono::Utc>> {
    db.notification(id).await.read_at
}

#[tokio::test]
async fn m1_concurrent_mark_read_under_different_branch_locks_returns_stored_read_at() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let id = db
        .notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;

    // tx A marks N read (uncommitted)
    let mut tx_a = db.begin().await;
    let read_at_a = service
        .mark_read_in(&mut tx_a, &user, &id)
        .await
        .expect("tx A mark_read_in");

    // tx B runs mark_read_in on a second connection, waiting on the row lock
    let service_b = db.service();
    let user_b = user.clone();
    let id_b = id.clone();
    let mut tx_b = db.begin().await;
    let handle_b = tokio::spawn(async move {
        let res = service_b.mark_read_in(&mut tx_b, &user_b, &id_b).await;
        let _ = tx_b.commit().await;
        res
    });

    // poll pg_stat_activity until a backend shows wait_event_type = 'Lock'
    let start = std::time::Instant::now();
    loop {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_stat_activity
                WHERE wait_event_type = 'Lock'
            )",
        )
        .fetch_one(&db.pool)
        .await
        .expect("query pg_stat_activity");

        if waiting {
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for tx B to block on row lock");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // A commits
    tx_a.commit().await.expect("commit tx A");

    // await B
    let res_b = handle_b.await.expect("join handle_b");
    let read_at_b = res_b.expect("tx B returns Ok(A's read_at)");
    assert_eq!(read_at_b, read_at_a);
    assert_eq!(read_at(&db, &id).await, Some(read_at_a));

    db.finish().await;
}

#[tokio::test]
async fn m2_unread_one_call_returns_new_read_at() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let id = db
        .notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;

    assert_eq!(read_at(&db, &id).await, None);
    let mut tx = db.begin().await;
    let read_at_val = service
        .mark_read_in(&mut tx, &user, &id)
        .await
        .expect("mark_read_in");
    tx.commit().await.expect("commit");

    assert_eq!(read_at(&db, &id).await, Some(read_at_val));
    db.finish().await;
}

#[tokio::test]
async fn m3_already_read_returns_stored_read_at_unchanged() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let id = db
        .notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;

    let mut tx1 = db.begin().await;
    let first = service
        .mark_read_in(&mut tx1, &user, &id)
        .await
        .expect("first mark_read_in");
    tx1.commit().await.expect("commit tx1");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut tx2 = db.begin().await;
    let second = service
        .mark_read_in(&mut tx2, &user, &id)
        .await
        .expect("second mark_read_in");
    tx2.commit().await.expect("commit tx2");

    assert_eq!(second, first, "a repeat answers the stored read_at");
    assert_eq!(read_at(&db, &id).await, Some(first), "read_at is unchanged");
    db.finish().await;
}

#[tokio::test]
async fn m4_another_users_notification_returns_out_of_scope() {
    let db = TestDb::start().await;
    let service = db.service();
    let (owner, _) = db.seed_customer_user("th").await;
    let (other, _) = db.seed_customer_user("th").await;
    let id = db
        .notify(&service, &request(&owner, "e:1", TRANSACTIONAL))
        .await;

    let mut tx = db.begin().await;
    let err = service
        .mark_read_in(&mut tx, &other, &id)
        .await
        .expect_err("another user's notification");
    assert_eq!(err.code(), "outOfScope");
    assert!(matches!(err, NotifyError::OutOfScope));
    drop(tx);
    assert_eq!(read_at(&db, &id).await, None);
    db.finish().await;
}

#[tokio::test]
async fn m5_unknown_id_returns_not_found() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let unknown_id = NotificationId(uuid::Uuid::new_v4().to_string());

    let mut tx = db.begin().await;
    let err = service
        .mark_read_in(&mut tx, &user, &unknown_id)
        .await
        .expect_err("unknown notification id");
    assert_eq!(err.code(), "notFound");
    assert!(matches!(err, NotifyError::NotFound));
    drop(tx);
    db.finish().await;
}
