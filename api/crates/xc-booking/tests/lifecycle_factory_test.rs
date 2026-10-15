//! #1863 (part 1 of #1641) — tests for `pg_booking_lifecycle` (design
//! `docs/design/blueprint/booking-slots.md` §5 `LifecycleFactory`, §7–9 rows F1–F3).

mod booking_common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use pos_error::AppError;
use uuid::Uuid;
use xc_booking::infrastructure::lifecycle_factory::pg_booking_lifecycle;
use xc_kernel::booking_queue_events::BookingQueueEvents;
use xc_kernel::ids::BranchId;

use booking_common::{ORG_1, TestDb, unique};

async fn seed_checked_in_booking(
    db: &TestDb,
    branch: &BranchId,
    customer_id: &str,
    slot_id: Uuid,
) -> String {
    let booking_id = format!("BK-{}", Uuid::new_v4());
    let booking_no = unique("B261015");
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
            'BUY', 'USD', 100, 'APP', 'CHECKED_IN', \
            decode('{qr_hex}', 'hex'), 1, now(), 'test', now(), now() \
        )",
        branch.as_str(),
    ))
    .await;
    booking_id
}

#[tokio::test]
async fn f1_on_ticket_serving_in_moves_checked_in_booking_to_in_service() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    let customer = db.add_customer("Alice", "0812345601").await;
    let now = Utc::now();
    let slot_id = db
        .add_slot(&branch, now + Duration::hours(1), 30, 2, 1)
        .await;
    let booking_id = seed_checked_in_booking(&db, &branch, &customer, slot_id).await;

    let lifecycle: Arc<dyn BookingQueueEvents> = pg_booking_lifecycle(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin tx");
    lifecycle
        .on_ticket_serving_in(&mut tx, &branch, &booking_id, now)
        .await
        .expect("on_ticket_serving_in succeeds");
    tx.commit().await.expect("commit tx");

    assert_eq!(
        db.text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .as_deref(),
        Some("IN_SERVICE")
    );

    db.finish().await;
}

#[tokio::test]
async fn f2_on_ticket_ended_day_closed_cancels_booking_and_releases_slot() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    let customer = db.add_customer("Bob", "0812345602").await;
    let now = Utc::now();
    let slot_id = db
        .add_slot(&branch, now + Duration::hours(1), 30, 2, 1)
        .await;
    let booking_id = seed_checked_in_booking(&db, &branch, &customer, slot_id).await;

    let lifecycle: Arc<dyn BookingQueueEvents> = pg_booking_lifecycle(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin tx");
    lifecycle
        .on_ticket_ended_in(&mut tx, &branch, &booking_id, "dayClosed", now)
        .await
        .expect("on_ticket_ended_in succeeds");
    tx.commit().await.expect("commit tx");

    assert_eq!(
        db.text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .as_deref(),
        Some("CANCELLED")
    );
    assert_eq!(
        db.text(&format!(
            "SELECT ended_reason FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .as_deref(),
        Some("dayClosed")
    );
    let (_, reserved, _) = db.slot_row(slot_id).await;
    assert_eq!(reserved, 0);

    db.finish().await;
}

#[tokio::test]
async fn f3_on_ticket_ended_day_closed_errors_when_slot_reserved_is_zero_and_leaves_booking_unchanged()
 {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    let customer = db.add_customer("Carol", "0812345603").await;
    let now = Utc::now();
    let slot_id = db
        .add_slot(&branch, now + Duration::hours(1), 30, 2, 0)
        .await;
    let booking_id = seed_checked_in_booking(&db, &branch, &customer, slot_id).await;

    let lifecycle: Arc<dyn BookingQueueEvents> = pg_booking_lifecycle(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin tx");
    let res = lifecycle
        .on_ticket_ended_in(&mut tx, &branch, &booking_id, "dayClosed", now)
        .await;
    assert!(
        matches!(res, Err(AppError::Conflict(ref code)) if code == "staleVersion"),
        "expected Conflict(staleVersion) from SlotRepository::release_in, got {res:?}"
    );
    tx.commit().await.expect("commit tx");

    assert_eq!(
        db.text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .as_deref(),
        Some("CHECKED_IN")
    );
    let (_, reserved, _) = db.slot_row(slot_id).await;
    assert_eq!(reserved, 0);

    db.finish().await;
}
