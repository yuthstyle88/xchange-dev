//! APP-3 (#993): `NotificationService::notify_in`, the producers' `Notifications` port (design
//! `customer-app.md` P8–P10, §4.1–§4.3, §9.1 row APP-3). Written before the implementation by the
//! test author (claude/dell-claude-2). Do not edit, delete or weaken; add tests in a file of your own.

mod common;

use common::*;

#[tokio::test]
async fn notify_in_twice_with_one_recipient_event_and_template_gives_one_row_and_the_same_id() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let req = request(&user, "queue.called:T-1:1", TRANSACTIONAL);

    let first = db.notify(&service, &req).await;
    let second = db.notify(&service, &req).await;

    assert_eq!(first, second, "a repeat answers the existing id");
    assert_eq!(db.notification_count(&user).await, 1);
    assert_eq!(
        db.delivery_count().await,
        1,
        "a repeat writes no second delivery"
    );
    db.finish().await;
}

#[tokio::test]
async fn another_event_for_the_same_recipient_and_template_is_a_new_notification() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;

    let first = db
        .notify(
            &service,
            &request(&user, "queue.called:T-1:1", TRANSACTIONAL),
        )
        .await;
    let second = db
        .notify(
            &service,
            &request(&user, "queue.called:T-1:2", TRANSACTIONAL),
        )
        .await;

    assert_ne!(first, second);
    assert_eq!(db.notification_count(&user).await, 2);
    db.finish().await;
}

#[tokio::test]
async fn the_notification_stores_the_request_the_template_version_and_the_recipients_language() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("en").await;

    let id = db
        .notify(
            &service,
            &request(&user, "queue.called:T-1:1", TRANSACTIONAL),
        )
        .await;

    let row = db.notification(&id).await;
    assert_eq!(row.recipient_user_id, user);
    assert_eq!(row.event_id, "queue.called:T-1:1");
    assert_eq!(row.template_key, TRANSACTIONAL);
    assert_eq!(row.template_version, TRANSACTIONAL_VERSION);
    assert_eq!(
        row.locale, "en",
        "the recipient's preferred_language at write time (P10)"
    );
    assert_eq!(row.object_type.as_deref(), Some("queue_ticket"));
    assert_eq!(row.object_id.as_deref(), Some("T-1"));
    assert_eq!(row.organization_id, ORG);
    assert_eq!(row.read_at, None);
    db.finish().await;
}

#[tokio::test]
async fn in_app_is_inserted_sent_in_the_producers_transaction() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;
    let req = request(&user, "queue.called:T-1:1", TRANSACTIONAL);

    let mut tx = db.begin().await;
    let id = service.notify_in(&mut tx, &req).await.expect("notify_in");
    assert_eq!(
        db.notification_count(&user).await,
        0,
        "nothing is visible before the producer commits: notify_in writes inside its transaction"
    );
    tx.commit().await.expect("commit");

    let deliveries = db.deliveries(&id).await;
    assert_eq!(
        deliveries,
        vec![DeliveryRow {
            channel: "IN_APP".into(),
            status: "SENT".into(),
            skipped_reason: None,
            organization_id: ORG.into(),
        }],
        "exactly one delivery, IN_APP, SENT (owner C3: no other channel)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_rollback_of_the_producers_transaction_writes_neither_row() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;

    let mut tx = db.begin().await;
    service
        .notify_in(
            &mut tx,
            &request(&user, "queue.called:T-1:1", TRANSACTIONAL),
        )
        .await
        .expect("notify_in");
    tx.rollback().await.expect("rollback");

    assert_eq!(db.notification_count(&user).await, 0);
    assert_eq!(db.delivery_count().await, 0);
    db.finish().await;
}

#[tokio::test]
async fn a_marketing_template_with_withdrawn_consent_is_skipped_with_no_consent() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, customer) = db.seed_customer_user("th").await;
    for channel in ["SMS", "EMAIL", "PUSH"] {
        db.record_marketing(&customer, channel, true, 0).await;
        db.record_marketing(&customer, channel, false, 60).await;
    }

    let id = db
        .notify(&service, &request(&user, "promo:1", MARKETING))
        .await;

    let deliveries = db.deliveries(&id).await;
    assert_eq!(
        deliveries.len(),
        1,
        "one IN_APP delivery row records the skip"
    );
    assert_eq!(deliveries[0].channel, "IN_APP");
    assert_eq!(deliveries[0].status, "SKIPPED");
    assert_eq!(deliveries[0].skipped_reason.as_deref(), Some("noConsent"));
    db.finish().await;
}

#[tokio::test]
async fn a_marketing_template_with_no_consent_ever_recorded_is_skipped_with_no_consent() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;

    let id = db
        .notify(&service, &request(&user, "promo:1", MARKETING))
        .await;

    let deliveries = db.deliveries(&id).await;
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, "SKIPPED");
    assert_eq!(deliveries[0].skipped_reason.as_deref(), Some("noConsent"));
    db.finish().await;
}

#[tokio::test]
async fn a_transactional_template_needs_no_marketing_consent() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, customer) = db.seed_customer_user("th").await;
    for channel in ["SMS", "EMAIL", "PUSH"] {
        db.record_marketing(&customer, channel, false, 0).await;
    }

    let id = db
        .notify(
            &service,
            &request(&user, "queue.called:T-1:1", TRANSACTIONAL),
        )
        .await;

    let deliveries = db.deliveries(&id).await;
    assert_eq!(deliveries.len(), 1);
    assert_eq!(
        deliveries[0].status, "SENT",
        "K5: transactional needs no marketing consent"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_unknown_template_key_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = db.service();
    let (user, _) = db.seed_customer_user("th").await;

    let mut tx = db.begin().await;
    let result = service
        .notify_in(&mut tx, &request(&user, "e:1", "no_such_template"))
        .await;
    assert!(result.is_err(), "a key with no template is refused");
    tx.commit()
        .await
        .expect("the producer's transaction is still usable: the key is checked before any write");

    assert_eq!(db.notification_count(&user).await, 0);
    assert_eq!(db.delivery_count().await, 0);
    db.finish().await;
}

/// Already guaranteed by APP-2's `NotificationRequest::new` (P10); kept here so the row of §9.1
/// APP-3 has a named test: a request whose params hold a phone never reaches `notify_in`.
#[test]
fn a_request_whose_params_hold_a_phone_is_refused_when_it_is_built() {
    let err = NotificationRequest::new(
        "6f1c1d2e-0000-4000-8000-000000000001",
        "queue.called:T-1:1",
        TRANSACTIONAL,
        None,
        params(&[("displayNumber", "W001"), ("phone", "0812345678")]),
    )
    .expect_err("a phone in params must be refused");
    assert_eq!(err, NotificationRequestError::PersonalData("phone".into()));
}
