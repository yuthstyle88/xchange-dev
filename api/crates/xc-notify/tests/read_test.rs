//! APP-3 (#993): the recipient's side of `NotificationService` — `markRead`, `markReadAll` and the
//! view `my_notifications` (design `customer-app.md` P10, P15, §4.2, §4.3, §4.4, §9.1 row APP-3).
//! Written before the implementation by the test author (claude/dell-claude-2). Do not edit,
//! delete or weaken; add tests in a file of your own.

mod common;

use std::time::Duration;

use common::*;

async fn read_at(db: &TestDb, id: &NotificationId) -> Option<chrono::DateTime<chrono::Utc>> {
    db.notification(id).await.read_at
}

fn changed(answer: MyNotifications) -> (i64, Vec<NotificationItem>) {
    match answer {
        MyNotifications::Changed { version, items } => (version, items),
        other => panic!("expected Changed, got {other:?}"),
    }
}

#[tokio::test]
async fn mark_read_twice_keeps_the_first_read_at() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let id = db
        .notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;

    let first = service.mark_read(&user, &id).await.expect("first markRead");
    assert_eq!(read_at(&db, &id).await, Some(first));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let second = service
        .mark_read(&user, &id)
        .await
        .expect("second markRead");

    assert_eq!(second, first, "a repeat answers the stored read_at");
    assert_eq!(read_at(&db, &id).await, Some(first), "read_at is set once");
    db.finish().await;
}

#[tokio::test]
async fn mark_read_of_another_users_notification_is_out_of_scope_and_changes_nothing() {
    let db = TestDb::start().await;
    let service = db.service();
    let (owner, _) = db.seed_customer_user("th").await;
    let (other, _) = db.seed_customer_user("th").await;
    let id = db
        .notify(&service, &request(&owner, "e:1", TRANSACTIONAL))
        .await;

    let err = service
        .mark_read(&other, &id)
        .await
        .expect_err("another user's notification");
    assert_eq!(err.code(), "outOfScope");
    assert_eq!(read_at(&db, &id).await, None);
    db.finish().await;
}

#[tokio::test]
async fn mark_all_read_marks_only_the_users_own_unread_notifications() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let (other, _) = db.seed_customer_user("th").await;
    let a = db
        .notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;
    let b = db
        .notify(&service, &request(&user, "e:2", TRANSACTIONAL))
        .await;
    let theirs = db
        .notify(&service, &request(&other, "e:1", TRANSACTIONAL))
        .await;
    let first_a = service.mark_read(&user, &a).await.expect("markRead a");

    let marked = service.mark_all_read(&user).await.expect("markReadAll");

    assert_eq!(marked, 1, "only b was unread");
    assert_eq!(
        read_at(&db, &a).await,
        Some(first_a),
        "a keeps its first read_at"
    );
    assert!(read_at(&db, &b).await.is_some());
    assert_eq!(
        read_at(&db, &theirs).await,
        None,
        "another user's row is untouched"
    );
    db.finish().await;
}

#[tokio::test]
async fn my_notifications_lists_only_the_users_own_rows_newest_first() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("en").await;
    let (other, _) = db.seed_customer_user("en").await;
    let older = db
        .notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;
    db.notify(&service, &request(&other, "e:1", TRANSACTIONAL))
        .await;
    let newer = db
        .notify(&service, &request(&user, "e:2", TRANSACTIONAL))
        .await;

    let (_, items) = changed(
        service
            .my_notifications(&user, &MyNotificationsQuery::default())
            .await
            .expect("my_notifications"),
    );

    let ids: Vec<&NotificationId> = items.iter().map(|i| &i.id).collect();
    assert_eq!(ids, vec![&newer, &older]);
    let item = &items[0];
    assert_eq!(item.title, "It is your turn");
    assert_eq!(item.body, "Queue W001");
    assert!(!item.read);
    assert_eq!(
        item.object,
        Some(NotificationObject {
            object_type: "queue_ticket".into(),
            object_id: "T-1".into(),
        })
    );
    db.finish().await;
}

#[tokio::test]
async fn the_unread_filter_leaves_out_read_notifications_and_the_read_flag_is_shown() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let read = db
        .notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;
    let unread = db
        .notify(&service, &request(&user, "e:2", TRANSACTIONAL))
        .await;
    service.mark_read(&user, &read).await.expect("markRead");

    let (_, all) = changed(
        service
            .my_notifications(&user, &MyNotificationsQuery::default())
            .await
            .expect("all"),
    );
    let flags: Vec<(NotificationId, bool)> = all.iter().map(|i| (i.id.clone(), i.read)).collect();
    assert_eq!(flags, vec![(unread.clone(), false), (read, true)]);

    let (_, only_unread) = changed(
        service
            .my_notifications(
                &user,
                &MyNotificationsQuery {
                    unread_only: true,
                    since: None,
                },
            )
            .await
            .expect("unread"),
    );
    let ids: Vec<NotificationId> = only_unread.into_iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![unread]);
    db.finish().await;
}

#[tokio::test]
async fn an_unchanged_since_answers_unchanged() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    db.notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;

    let (version, _) = changed(
        service
            .my_notifications(&user, &MyNotificationsQuery::default())
            .await
            .expect("first read"),
    );
    let again = service
        .my_notifications(
            &user,
            &MyNotificationsQuery {
                unread_only: false,
                since: Some(version),
            },
        )
        .await
        .expect("second read");

    assert_eq!(again, MyNotifications::Unchanged { version });
    db.finish().await;
}

#[tokio::test]
async fn another_users_new_notification_does_not_change_my_version() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let (other, _) = db.seed_customer_user("th").await;
    db.notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;
    let (version, _) = changed(
        service
            .my_notifications(&user, &MyNotificationsQuery::default())
            .await
            .expect("first read"),
    );

    db.notify(&service, &request(&other, "e:1", TRANSACTIONAL))
        .await;
    let again = service
        .my_notifications(
            &user,
            &MyNotificationsQuery {
                unread_only: false,
                since: Some(version),
            },
        )
        .await
        .expect("second read");

    assert_eq!(
        again,
        MyNotifications::Unchanged { version },
        "the version is per recipient (P15)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_new_notification_after_since_answers_changed_with_a_higher_version() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    db.notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;
    let (version, _) = changed(
        service
            .my_notifications(&user, &MyNotificationsQuery::default())
            .await
            .expect("first read"),
    );

    let newer = db
        .notify(&service, &request(&user, "e:2", TRANSACTIONAL))
        .await;
    let (next, items) = changed(
        service
            .my_notifications(
                &user,
                &MyNotificationsQuery {
                    unread_only: false,
                    since: Some(version),
                },
            )
            .await
            .expect("second read"),
    );

    assert!(next > version);
    assert!(items.iter().any(|i| i.id == newer));
    db.finish().await;
}

#[tokio::test]
async fn a_zh_row_renders_in_chinese_after_the_reader_switches_to_thai() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, customer) = db.seed_customer_user("zh").await;
    db.notify(&service, &request(&user, "e:1", TRANSACTIONAL))
        .await;
    db.set_preferred_language(&customer, "th").await;

    let (_, items) = changed(
        service
            .my_notifications(&user, &MyNotificationsQuery::default())
            .await
            .expect("my_notifications"),
    );

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].locale, Locale::Zh, "the stored locale (P10)");
    assert_eq!(items[0].title, "轮到您了");
    assert_eq!(items[0].body, "排队号 W001");
    db.finish().await;
}
