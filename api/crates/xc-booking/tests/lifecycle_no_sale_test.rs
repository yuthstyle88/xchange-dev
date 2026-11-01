//! Triage T1 (2026-11-01 real-staff e2e, step 4, booking `B261101-0009`): a booking whose ticket a teller released
//! with no sale (`noSale`, queue-tickets §4.6, #5541) must end `CANCELLED (noSale)` and give its slot place back, as
//! a `dayClosed` ticket end does (`lifecycle_factory_test.rs` F2). Today `BookingLifecycle::on_ticket_ended_in`
//! ignores every reason but `dayClosed` (`lifecycle.rs`), so the booking stays `IN_SERVICE` for good.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod booking_common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use uuid::Uuid;
use xc_booking::infrastructure::lifecycle_factory::pg_booking_lifecycle;
use xc_kernel::booking_queue_events::BookingQueueEvents;
use xc_kernel::ids::BranchId;

use booking_common::{ORG_1, TestDb, unique};

async fn seed_booking(
    db: &TestDb,
    branch: &BranchId,
    customer_id: &str,
    slot_id: Uuid,
    status: &str,
) -> String {
    let booking_id = format!("BK-{}", Uuid::new_v4());
    let booking_no = unique("B261101");
    let qr_hex = format!(
        "{:032x}{:032x}",
        Uuid::new_v4().as_u128(),
        Uuid::new_v4().as_u128()
    );
    db.exec(&format!(
        "INSERT INTO bookings ( \
            id, organization_id, branch_id, booking_no, customer_id, slot_id, \
            shop_side, requested_currency, requested_quantity, source_channel, status, \
            qr_token_hash, qr_version, checked_in_at, created_by, created_at, updated_at \
        ) VALUES ( \
            '{booking_id}', '{ORG_1}', '{}', '{booking_no}', '{customer_id}', '{slot_id}', \
            'BUY', 'USD', 100, 'APP', '{status}', \
            decode('{qr_hex}', 'hex'), 1, now(), 'test', now(), now() \
        )",
        branch.as_str(),
    ))
    .await;
    booking_id
}

async fn end(db: &TestDb, branch: &BranchId, booking_id: &str, reason: &str) {
    let lifecycle: Arc<dyn BookingQueueEvents> = pg_booking_lifecycle(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin tx");
    lifecycle
        .on_ticket_ended_in(&mut tx, branch, booking_id, reason, Utc::now())
        .await
        .expect("on_ticket_ended_in succeeds");
    tx.commit().await.expect("commit tx");
}

async fn status_and_reason(db: &TestDb, booking_id: &str) -> String {
    db.text(&format!(
        "SELECT status || ' ' || coalesce(ended_reason, '-') FROM bookings WHERE id = '{booking_id}'"
    ))
    .await
    .unwrap_or_default()
}

async fn cancel_events(db: &TestDb, booking_id: &str) -> Vec<String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT from_status || '>' || to_status || ' ' || coalesce(reason, '-') FROM booking_events \
         WHERE booking_id = $1 AND kind = 'CANCEL' ORDER BY at",
    )
    .bind(booking_id)
    .fetch_all(&db.pool)
    .await
    .expect("read booking events");
    rows.into_iter().map(|(s,)| s).collect()
}

#[tokio::test]
async fn t1_no_sale_cancels_an_in_service_booking_and_releases_its_slot() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    let customer = db.add_customer("Dan", "0812345611").await;
    let slot_id = db
        .add_slot(&branch, Utc::now() + Duration::hours(1), 30, 2, 1)
        .await;
    let booking_id = seed_booking(&db, &branch, &customer, slot_id, "IN_SERVICE").await;

    end(&db, &branch, &booking_id, "noSale").await;

    assert_eq!(
        status_and_reason(&db, &booking_id).await,
        "CANCELLED noSale"
    );
    assert_eq!(
        cancel_events(&db, &booking_id).await,
        ["IN_SERVICE>CANCELLED noSale"],
        "one CANCEL booking event with the reason"
    );
    let (_, reserved, _) = db.slot_row(slot_id).await;
    assert_eq!(reserved, 0, "the place goes back to the slot");
    db.finish().await;
}

#[tokio::test]
async fn t1_no_sale_leaves_an_already_ended_booking_unchanged() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    let customer = db.add_customer("Eve", "0812345612").await;
    let slot_id = db
        .add_slot(&branch, Utc::now() + Duration::hours(1), 30, 2, 1)
        .await;
    let booking_id = seed_booking(&db, &branch, &customer, slot_id, "CANCELLED").await;

    end(&db, &branch, &booking_id, "noSale").await;

    assert_eq!(status_and_reason(&db, &booking_id).await, "CANCELLED -");
    assert_eq!(cancel_events(&db, &booking_id).await, Vec::<String>::new());
    let (_, reserved, _) = db.slot_row(slot_id).await;
    assert_eq!(reserved, 1, "an ended booking gives no place back twice");
    db.finish().await;
}

#[tokio::test]
async fn t1_other_reasons_still_leave_the_booking_unchanged() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    let customer = db.add_customer("Fay", "0812345613").await;
    let slot_id = db
        .add_slot(&branch, Utc::now() + Duration::hours(1), 30, 2, 1)
        .await;
    let booking_id = seed_booking(&db, &branch, &customer, slot_id, "IN_SERVICE").await;

    end(&db, &branch, &booking_id, "customerLeft").await;

    assert_eq!(status_and_reason(&db, &booking_id).await, "IN_SERVICE -");
    let (_, reserved, _) = db.slot_row(slot_id).await;
    assert_eq!(reserved, 1);
    db.finish().await;
}
