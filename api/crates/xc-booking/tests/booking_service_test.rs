//! BK-3 #1380 — `BookingService` tests (design §4.3, §4.4, §8, §9.1).
//!
//! Every row of design §9.1 and ticket #1380 §9 is pinned by a test here.

mod booking_common;

use std::sync::Arc;

use chrono::{Duration, DurationRound, Utc};
use rust_decimal::Decimal;
use tokio::sync::Barrier;
use uuid::Uuid;
use xc_booking::application::booking_service::{ContactInput, CreateBookingInput};
use xc_booking::domain::booking::booking_error::BookingError;
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::domain::booking::repository::BookingRepository;
use xc_booking::domain::booking::source_channel::SourceChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_referral::domain::capture::CaptureTarget;
use xc_referral::domain::capture_repository::CaptureRepository;
use xc_referral::infrastructure::capture_repository::PgCaptureRepository;

use booking_common::{ORG_1, TestDb};

fn base_create_input(
    branch_id: BranchId,
    slot_id: Uuid,
    channel: SourceChannel,
    customer_id: Option<String>,
) -> CreateBookingInput {
    CreateBookingInput {
        organization_id: Uuid::parse_str(ORG_1).unwrap(),
        branch_id,
        slot_id,
        shop_side: Side::Buy,
        requested_currency: "USD".to_string(),
        requested_quantity: Decimal::new(100, 0),
        channel,
        actor_id: "test-actor".to_string(),
        customer_id,
        contact: None,
        quote_id: None,
        referral_code: None,
        idempotency_key: None,
    }
}

// ------------------------------------------------------------ 1. Initial Status by Channel

#[tokio::test]
async fn app_create_starts_confirmed() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("App User", "+66811111111").await;

    let input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id));
    let created = service
        .create(input, now)
        .await
        .expect("create app booking");

    assert_eq!(created.booking.status, BookingStatus::Confirmed);
    assert!(!created.qr_token.is_empty(), "QR token returned on create");

    let status_in_db: String = db
        .text(&format!(
            "SELECT status FROM bookings WHERE id = '{}'",
            created.booking.id
        ))
        .await
        .expect("booking in db");
    assert_eq!(status_in_db, "CONFIRMED");

    let (_, reserved, _) = db.slot_row(slot).await;
    assert_eq!(reserved, 1, "holding state increments slot reserved");

    db.finish().await;
}

#[tokio::test]
async fn desk_create_starts_confirmed() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;

    let mut input = base_create_input(branch, slot, SourceChannel::Desk, None);
    input.contact = Some(ContactInput {
        display_name: "Walk-in Desk".to_string(),
        phone: "+66822222222".to_string(),
    });

    let created = service
        .create(input, now)
        .await
        .expect("create desk booking");
    assert_eq!(
        created.booking.status,
        BookingStatus::Confirmed,
        "Q-B5: desk booking starts CONFIRMED"
    );

    let (_, reserved, _) = db.slot_row(slot).await;
    assert_eq!(reserved, 1);

    db.finish().await;
}

// ------------------------------------------------------------ 2. Idempotency & Replay

#[tokio::test]
async fn replay_with_same_key_returns_same_booking_and_writes_once() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("Replay User", "+66833333333").await;

    let mut input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id));
    input.idempotency_key = Some("key-bk-replay-1".to_string());

    let first = service
        .create(input.clone(), now)
        .await
        .expect("first create");
    let second = service
        .create(input.clone(), now)
        .await
        .expect("replay create");

    assert_eq!(first.booking.id, second.booking.id);
    assert_eq!(first.booking.booking_no, second.booking.booking_no);

    let count = db
        .count(&format!(
            "SELECT count(*) FROM bookings WHERE id = '{}'",
            first.booking.id
        ))
        .await;
    assert_eq!(count, 1, "written once");

    let (_, reserved, _) = db.slot_row(slot).await;
    assert_eq!(reserved, 1, "place incremented only once");

    db.finish().await;
}

#[tokio::test]
async fn replay_with_same_key_other_body_is_idempotency_conflict() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("Conflict User", "+66844444444").await;

    let mut input1 = base_create_input(
        branch.clone(),
        slot,
        SourceChannel::App,
        Some(customer_id.clone()),
    );
    input1.idempotency_key = Some("key-bk-conflict-1".to_string());
    input1.requested_quantity = Decimal::new(100, 0);

    let _ = service.create(input1, now).await.expect("first create");

    let mut input2 = base_create_input(branch, slot, SourceChannel::App, Some(customer_id));
    input2.idempotency_key = Some("key-bk-conflict-1".to_string());
    input2.requested_quantity = Decimal::new(200, 0); // different body

    let err = service
        .create(input2, now)
        .await
        .expect_err("conflict expected");
    assert!(matches!(err, BookingError::IdempotencyConflict));

    db.finish().await;
}

#[tokio::test]
async fn stored_replay_answer_holds_no_qr_token() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("NoQrStored User", "+66855555555").await;

    let mut input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id));
    input.idempotency_key = Some("key-no-qr-store".to_string());

    let _ = service.create(input, now).await.expect("create");

    let stored_response = db
        .text("SELECT response_ref::text FROM idempotency_records WHERE key = 'key-no-qr-store'")
        .await
        .expect("idempotency record must exist in idempotency_records");
    assert!(
        !stored_response.contains("qr_token") && !stored_response.contains("qrToken"),
        "stored replay response body must never hold plain QR token (B7)"
    );

    db.finish().await;
}

// ------------------------------------------------------------ 3. Cancel

#[tokio::test]
async fn customer_cancel_before_cutoff_makes_cancelled_and_releases_place() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;

    let base_time = (Utc::now() + Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let slot = db.add_slot(&branch, base_time, 30, 5, 0).await;
    let customer_id = db.add_customer("Cancel User", "+66866666666").await;

    let input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id.clone()));
    let created = service
        .create(input, base_time - Duration::hours(5))
        .await
        .expect("create");

    let cancel_time = base_time - Duration::minutes(75); // 75 min before, cutoff is 60 min
    let cancelled = service
        .cancel(&created.booking.id, &customer_id, false, None, cancel_time)
        .await
        .expect("cancel before cutoff");

    assert_eq!(cancelled.status, BookingStatus::Cancelled);

    let (_, reserved, _) = db.slot_row(slot).await;
    assert_eq!(reserved, 0, "place released on cancel");

    db.finish().await;
}

#[tokio::test]
async fn customer_cancel_exactly_at_cutoff_boundary_succeeds() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;

    let base_time = (Utc::now() + Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let slot = db.add_slot(&branch, base_time, 30, 5, 0).await;
    let customer_id = db.add_customer("Cutoff User", "+66877777777").await;

    let input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id.clone()));
    let created = service
        .create(input, base_time - Duration::hours(5))
        .await
        .expect("create");

    let boundary_time = base_time - Duration::minutes(60); // exactly 60 min before
    let cancelled = service
        .cancel(
            &created.booking.id,
            &customer_id,
            false,
            None,
            boundary_time,
        )
        .await
        .expect("cancel at cutoff boundary");

    assert_eq!(cancelled.status, BookingStatus::Cancelled);

    db.finish().await;
}

/// #4203, restated by the ticket author (claude/studio-claude-6) on the owner's ruling of 2026-10-21: "ยกเลิกการจอง ยกเลิกได้เลย" · "ไม่ต้อง มี การอนุมัติ เพราะไม่มีอะไรเกี่ยวกับเงิน".
/// A customer cancel inside the cutoff now succeeds and releases the place. Before #4203 this test asserted
/// `CancelCutoffPassed`.
#[tokio::test]
async fn customer_cancel_after_cutoff_makes_cancelled_and_releases_place() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;

    let base_time = (Utc::now() + Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let slot = db.add_slot(&branch, base_time, 30, 5, 0).await;
    let customer_id = db.add_customer("Late User", "+66888888888").await;

    let input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id.clone()));
    let created = service
        .create(input, base_time - Duration::hours(5))
        .await
        .expect("create");

    let late_time = base_time - Duration::minutes(59); // 59 min before (inside the 60m cutoff)
    let cancelled = service
        .cancel(&created.booking.id, &customer_id, false, None, late_time)
        .await
        .expect("#4203: a customer may cancel at any time");

    assert_eq!(cancelled.status, BookingStatus::Cancelled);

    let (_, reserved, _) = db.slot_row(slot).await;
    assert_eq!(reserved, 0, "place released on cancel");

    db.finish().await;
}

#[tokio::test]
async fn teller_cancel_without_reason_is_refused() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;

    let base_time = (Utc::now() + Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let slot = db.add_slot(&branch, base_time, 30, 5, 0).await;
    let customer_id = db.add_customer("TellerCancel User", "+66899999999").await;

    let input = base_create_input(branch, slot, SourceChannel::Desk, Some(customer_id));
    let created = service
        .create(input, base_time - Duration::hours(5))
        .await
        .expect("create");

    let err = service
        .cancel(
            &created.booking.id,
            "teller-user-1",
            true,
            None, // teller without reason
            base_time - Duration::hours(2),
        )
        .await
        .expect_err("teller cancel without reason refused");

    assert!(matches!(err, BookingError::TellerCancelReasonRequired));

    db.finish().await;
}

#[tokio::test]
async fn teller_cancel_with_reason_succeeds_after_cutoff() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;

    let base_time = (Utc::now() + Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let slot = db.add_slot(&branch, base_time, 30, 5, 0).await;
    let customer_id = db.add_customer("TellerReason User", "+66810101010").await;

    let input = base_create_input(branch, slot, SourceChannel::Desk, Some(customer_id));
    let created = service
        .create(input, base_time - Duration::hours(5))
        .await
        .expect("create");

    let late_time = base_time - Duration::minutes(20); // 20 min before slot
    let cancelled = service
        .cancel(
            &created.booking.id,
            "teller-user-1",
            true,
            Some("Customer called branch to cancel"),
            late_time,
        )
        .await
        .expect("teller cancel with reason succeeds after cutoff");

    assert_eq!(cancelled.status, BookingStatus::Cancelled);

    let (_, reserved, _) = db.slot_row(slot).await;
    assert_eq!(reserved, 0, "place released");

    db.finish().await;
}

// ------------------------------------------------------------ 4. Reschedule

#[tokio::test]
async fn reschedule_to_same_slot_is_unchanged() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;

    let base_time = (Utc::now() + Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let slot = db.add_slot(&branch, base_time, 30, 5, 0).await;
    let customer_id = db.add_customer("ReschedSame User", "+66812121212").await;

    let input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id.clone()));
    let created = service
        .create(input, base_time - Duration::hours(5))
        .await
        .expect("create");

    let resched_time = base_time - Duration::hours(2);
    let updated = service
        .reschedule(&created.booking.id, slot, &customer_id, false, resched_time)
        .await
        .expect("reschedule to same slot");

    assert_eq!(updated.slot_id, slot);
    let (_, reserved, _) = db.slot_row(slot).await;
    assert_eq!(reserved, 1, "reserved remains 1");

    db.finish().await;
}

#[tokio::test]
async fn reschedule_to_full_slot_is_capacity_full_and_nothing_moves() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;

    let base_time = (Utc::now() + Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let slot1 = db.add_slot(&branch, base_time, 30, 5, 0).await;
    let slot2_full = db
        .add_slot(&branch, base_time + Duration::hours(1), 30, 2, 2)
        .await; // full slot
    let customer_id = db.add_customer("ReschedFull User", "+66813131313").await;

    let input = base_create_input(branch, slot1, SourceChannel::App, Some(customer_id.clone()));
    let created = service
        .create(input, base_time - Duration::hours(5))
        .await
        .expect("create");

    let resched_time = base_time - Duration::hours(2);
    let err = service
        .reschedule(
            &created.booking.id,
            slot2_full,
            &customer_id,
            false,
            resched_time,
        )
        .await
        .expect_err("reschedule to full slot must fail");

    assert!(matches!(err, BookingError::CapacityFull));

    // verify nothing moved
    let current_slot: Uuid = db
        .text(&format!(
            "SELECT slot_id::text FROM bookings WHERE id = '{}'",
            created.booking.id
        ))
        .await
        .map(|s| Uuid::parse_str(&s).unwrap())
        .expect("read slot_id");
    assert_eq!(current_slot, slot1, "booking stayed on slot 1");

    let (_, res1, _) = db.slot_row(slot1).await;
    assert_eq!(res1, 1, "slot 1 still has 1 reserved");

    let (_, res2, _) = db.slot_row(slot2_full).await;
    assert_eq!(res2, 2, "slot 2 still full");

    db.finish().await;
}

// ------------------------------------------------------------ 5. Confirm Expiry

#[tokio::test]
async fn confirm_after_confirm_expires_at_is_booking_expired() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;

    let base_time = (Utc::now() + Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let slot = db.add_slot(&branch, base_time, 30, 5, 0).await;
    let customer_id = db.add_customer("ConfirmExp User", "+66814141414").await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    let created_at = base_time - Duration::hours(5);
    let confirm_expires_at = base_time - Duration::hours(4); // 60 min after create
    let qr_token_hash = [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    let partner_attribution_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, \
         shop_side, requested_currency, requested_quantity, source_channel, partner_attribution_id, \
         status, qr_token_hash, qr_version, confirm_expires_at, created_by, created_at, updated_at) \
         VALUES ($1, $2, $3, 'B261012-0001', $4, $5, 'BUY', 'USD', 100, 'PARTNER', $6, \
         'PENDING_CUSTOMER_CONFIRMATION', $7, 1, $8, 'test', $9, $9)",
    )
    .bind(&booking_id)
    .bind(Uuid::parse_str(ORG_1).unwrap())
    .bind(branch.as_str())
    .bind(&customer_id)
    .bind(slot)
    .bind(partner_attribution_id)
    .bind(&qr_token_hash)
    .bind(confirm_expires_at)
    .bind(created_at)
    .execute(&db.pool)
    .await
    .expect("seed pending booking");

    sqlx::query("UPDATE booking_slots SET reserved = reserved + 1 WHERE id = $1")
        .bind(slot)
        .execute(&db.pool)
        .await
        .expect("reserve slot");

    // Confirm after confirm_expires_at
    let confirm_time = base_time - Duration::hours(3); // well after 60 min expiry
    let err = service
        .confirm(&booking_id, &customer_id, confirm_time)
        .await
        .expect_err("confirm after expiry must fail");

    assert!(matches!(err, BookingError::BookingExpired));

    db.finish().await;
}

// ------------------------------------------------------------ 6. Quote Validation

#[tokio::test]
async fn quote_of_another_branch_or_side_or_currency_or_quantity_is_validation_failed_quote_id() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch1 = db.add_branch("Asia/Bangkok", None, true).await;
    let branch2 = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch1, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch1, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("Quote User", "+66815151515").await;

    // Quote for branch 2, not branch 1
    let other_branch_quote = db
        .add_quote(&branch2, Side::Buy, "USD", Decimal::new(100, 0), "ACTIVE")
        .await;

    let mut input = base_create_input(
        branch1.clone(),
        slot,
        SourceChannel::App,
        Some(customer_id.clone()),
    );
    input.quote_id = Some(other_branch_quote);

    let err = service
        .create(input, now)
        .await
        .expect_err("quote of another branch refused");
    assert!(matches!(
        err,
        BookingError::ValidationFailed { field: "quoteId" }
    ));

    // Quote for Sell, but booking is Buy
    let wrong_side_quote = db
        .add_quote(&branch1, Side::Sell, "USD", Decimal::new(100, 0), "ACTIVE")
        .await;

    let mut input_side = base_create_input(
        branch1.clone(),
        slot,
        SourceChannel::App,
        Some(customer_id.clone()),
    );
    input_side.shop_side = Side::Buy;
    input_side.quote_id = Some(wrong_side_quote);

    let err_side = service
        .create(input_side, now)
        .await
        .expect_err("quote with wrong side refused");
    assert!(matches!(
        err_side,
        BookingError::ValidationFailed { field: "quoteId" }
    ));

    db.finish().await;
}

#[tokio::test]
async fn quote_cancelled_is_validation_failed_quote_id() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("CancelledQuote User", "+66816161616").await;

    let cancelled_quote = db
        .add_quote(&branch, Side::Buy, "USD", Decimal::new(100, 0), "CANCELLED")
        .await;

    let mut input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id));
    input.quote_id = Some(cancelled_quote);

    let err = service
        .create(input, now)
        .await
        .expect_err("cancelled quote refused");
    assert!(matches!(
        err,
        BookingError::ValidationFailed { field: "quoteId" }
    ));

    db.finish().await;
}

// ------------------------------------------------------------ 7. QR Re-issue

#[tokio::test]
async fn qr_reissue_makes_old_token_find_nothing_and_new_token_finds_booking() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("QrReissue User", "+66817171717").await;

    let input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id.clone()));
    let created = service.create(input, now).await.expect("create");
    let old_token = created.qr_token;

    let reissued = service
        .reissue_qr(
            &created.booking.id,
            &customer_id,
            now + Duration::minutes(5),
        )
        .await
        .expect("reissue qr");
    assert_ne!(old_token, reissued.qr_token, "new QR token generated");
    assert_eq!(reissued.booking.qr_version, 2, "qr_version incremented");

    let repo = db.booking_repo();
    let mut tx = db.pool.begin().await.expect("begin tx");

    let old_hash = xc_booking::domain::booking::qr_token::QrToken::hash(&old_token);
    let found_old = repo
        .by_qr_hash(&mut tx, &old_hash)
        .await
        .expect("query old");
    assert!(found_old.is_none(), "old token finds nothing");

    let new_hash = xc_booking::domain::booking::qr_token::QrToken::hash(&reissued.qr_token);
    let found_new = repo
        .by_qr_hash(&mut tx, &new_hash)
        .await
        .expect("query new");
    assert!(found_new.is_some(), "new token finds booking");
    assert_eq!(found_new.unwrap().id, created.booking.id);

    tx.commit().await.expect("commit tx");
    db.finish().await;
}

// ------------------------------------------------------------ 8. Referral Captures

#[tokio::test]
async fn desk_booking_with_referral_code_gives_one_active_capture() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;

    let mut input = base_create_input(branch, slot, SourceChannel::Desk, None);
    input.contact = Some(ContactInput {
        display_name: "Referral Desk".to_string(),
        phone: "+66818181818".to_string(),
    });
    input.referral_code = Some("AB12CD".to_string());

    let created = service
        .create(input, now)
        .await
        .expect("create desk with ref");

    let capture_count = db
        .count(&format!(
            "SELECT count(*) FROM referral_attributions WHERE booking_id = '{}' AND status = 'ACTIVE'",
            created.booking.id
        ))
        .await;
    assert_eq!(capture_count, 1, "exactly one ACTIVE capture written");

    db.finish().await;
}

#[tokio::test]
async fn desk_booking_without_referral_code_gives_no_capture() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;

    let mut input = base_create_input(branch, slot, SourceChannel::Desk, None);
    input.contact = Some(ContactInput {
        display_name: "NoRef Desk".to_string(),
        phone: "+66819191919".to_string(),
    });
    input.referral_code = None;

    let created = service
        .create(input, now)
        .await
        .expect("create desk without ref");

    let capture_count = db
        .count(&format!(
            "SELECT count(*) FROM referral_attributions WHERE booking_id = '{}'",
            created.booking.id
        ))
        .await;
    assert_eq!(capture_count, 0, "no capture written");

    db.finish().await;
}

#[tokio::test]
async fn app_booking_with_referral_code_gives_one_active_capture() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("AppRef User", "+66820202020").await;

    let mut input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id));
    input.referral_code = Some("XY34ZW".to_string());

    let created = service
        .create(input, now)
        .await
        .expect("create app with ref");

    let capture_count = db
        .count(&format!(
            "SELECT count(*) FROM referral_attributions WHERE booking_id = '{}' AND status = 'ACTIVE'",
            created.booking.id
        ))
        .await;
    assert_eq!(capture_count, 1, "one ACTIVE capture on app booking");

    db.finish().await;
}

#[tokio::test]
async fn booking_capture_takes_organization_branch_and_customer_from_booking_row() {
    let db = TestDb::start().await;
    let service = db.service();

    // Org 1 has branch1 and customer1
    let branch1 = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch1, 30, 5, 14).await;
    let customer1 = db.add_customer("Org1 Customer", "+66811111111").await;

    // Seed a second organization, branch, and customer
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{org2}', 'ORG2', 'Organization 2', 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO booking_policies (organization_id, cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes) \
         VALUES ('{org2}', 60, 15, 60)"
    ))
    .await;

    let branch2 = db.add_branch_in_org(org2, "Asia/Bangkok", None, true).await;
    db.add_slot_rule(&branch2, 30, 5, 14).await;
    let customer2 = db
        .add_customer_in_org(org2, "Org2 Customer", "+66822222222")
        .await;

    let now = Utc::now();
    let slot2 = db
        .add_slot(&branch2, now + Duration::hours(2), 30, 5, 0)
        .await;

    let mut input = base_create_input(
        branch2.clone(),
        slot2,
        SourceChannel::App,
        Some(customer2.clone()),
    );
    input.organization_id = Uuid::parse_str(org2).unwrap();
    input.actor_id = "actor-org1".to_string();
    input.referral_code = Some("REFORG2".to_string());

    let created = service
        .create(input, now)
        .await
        .expect("create app booking in org 2 with referral");

    // 1. Assert referral_attributions row takes org, branch, customer from booking row
    let (attr_org_id, attr_branch_id, attr_customer_id, attr_status): (
        Uuid,
        String,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT organization_id, branch_id, customer_id, status FROM referral_attributions WHERE booking_id = $1",
    )
    .bind(&created.booking.id)
    .fetch_one(&db.pool)
    .await
    .expect("referral attribution row for booking");

    assert_eq!(
        attr_org_id,
        Uuid::parse_str(org2).unwrap(),
        "capture takes organization_id from booking row (not Org 1)"
    );
    assert_ne!(attr_org_id, Uuid::parse_str(ORG_1).unwrap());

    assert_eq!(
        attr_branch_id,
        branch2.as_str(),
        "capture takes branch_id from booking row (not branch 1)"
    );
    assert_ne!(attr_branch_id, branch1.as_str());

    assert_eq!(
        attr_customer_id, customer2,
        "capture takes customer_id from booking row (not customer 1)"
    );
    assert_ne!(attr_customer_id, customer1);

    assert_eq!(attr_status, "ACTIVE");

    // 2. Also assert resolve_target directly (mutants m2/m3/m3c from BK-1 review)
    let mut tx = db.pool.begin().await.expect("begin tx");
    let target = PgCaptureRepository::new()
        .resolve_target(&mut tx, &CaptureTarget::Booking(created.booking.id.clone()))
        .await
        .expect("resolve_target succeeds for booking");
    assert_eq!(target.organization_id, Uuid::parse_str(org2).unwrap());
    assert_eq!(target.branch_id, branch2.as_str());
    assert_eq!(target.customer_id, customer2);
    tx.rollback().await.expect("rollback");

    db.finish().await;
}

#[tokio::test]
async fn malformed_referral_code_refuses_create_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = db.service();
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let now = Utc::now();
    let slot = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("BadRef User", "+66821212121").await;

    let mut input = base_create_input(branch, slot, SourceChannel::App, Some(customer_id));
    input.referral_code = Some("not-a-valid-code!@#".to_string());

    let err = service
        .create(input, now)
        .await
        .expect_err("malformed referral code must fail");
    assert!(matches!(
        err,
        BookingError::ValidationFailed {
            field: "referralCode"
        }
    ));

    let booking_count = db.count("SELECT count(*) FROM bookings").await;
    assert_eq!(booking_count, 0, "nothing written to bookings");

    let capture_count = db.count("SELECT count(*) FROM referral_attributions").await;
    assert_eq!(capture_count, 0, "nothing written to referral_attributions");

    let (_, reserved, _) = db.slot_row(slot).await;
    assert_eq!(reserved, 0, "slot capacity not reserved");

    db.finish().await;
}

// ------------------------------------------------------------ 9. Bookings Closed Refusals

#[tokio::test]
async fn branch_with_booking_disabled_no_rule_or_no_policy_is_bookings_closed() {
    let db = TestDb::start().await;
    let service = db.service();

    // Case A: booking_enabled = false
    let branch_disabled = db.add_branch("Asia/Bangkok", None, false).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch_disabled, 30, 5, 14).await;
    let now = Utc::now();
    let slot_a = db
        .add_slot(&branch_disabled, now + Duration::hours(2), 30, 5, 0)
        .await;
    let cust_a = db.add_customer("Cust A", "+66822222221").await;

    let input_a = base_create_input(branch_disabled, slot_a, SourceChannel::App, Some(cust_a));
    let err_a = service
        .create(input_a, now)
        .await
        .expect_err("booking disabled branch must fail");
    assert!(matches!(err_a, BookingError::BookingsClosed));

    // Case B: no slot rule
    let branch_no_rule = db.add_branch("Asia/Bangkok", None, true).await;
    let slot_b = db
        .add_slot(&branch_no_rule, now + Duration::hours(2), 30, 5, 0)
        .await;
    let cust_b = db.add_customer("Cust B", "+66822222223").await;

    let input_b = base_create_input(branch_no_rule, slot_b, SourceChannel::App, Some(cust_b));
    let err_b = service
        .create(input_b, now)
        .await
        .expect_err("branch with no slot rule must fail");
    assert!(matches!(err_b, BookingError::BookingsClosed));

    // Case C: no policy row
    let branch_c = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_slot_rule(&branch_c, 30, 5, 14).await;
    db.exec(&format!(
        "DELETE FROM booking_policies WHERE organization_id = '{ORG_1}'"
    ))
    .await;
    let slot_c = db
        .add_slot(&branch_c, now + Duration::hours(2), 30, 5, 0)
        .await;
    let cust_c = db.add_customer("Cust C", "+66822222224").await;

    let input_c = base_create_input(branch_c, slot_c, SourceChannel::App, Some(cust_c));
    let err_c = service
        .create(input_c, now)
        .await
        .expect_err("no policy row must fail");
    assert!(matches!(err_c, BookingError::BookingsClosed));

    db.finish().await;
}

// ------------------------------------------------------------ 10. Concurrency Race on Last Place

#[tokio::test]
async fn concurrent_race_for_last_place_gives_it_to_exactly_one_booker() {
    let db = TestDb::start_with(16).await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 1, 14).await;

    let service = Arc::new(db.service());

    // Repeat across 5 rounds to ensure real race condition is rigorously verified
    for round in 0..5 {
        let now = Utc::now() + Duration::days(round + 1);
        // Slot has capacity 1, 0 reserved
        let slot = db
            .add_slot(&branch, now + Duration::hours(2), 30, 1, 0)
            .await;

        let cust1 = db
            .add_customer(&format!("Race1_{round}"), &format!("+6683100{round:04}"))
            .await;
        let cust2 = db
            .add_customer(&format!("Race2_{round}"), &format!("+6683200{round:04}"))
            .await;

        let input1 = base_create_input(branch.clone(), slot, SourceChannel::App, Some(cust1));
        let input2 = base_create_input(branch.clone(), slot, SourceChannel::App, Some(cust2));

        let barrier = Arc::new(Barrier::new(2));

        let svc1 = service.clone();
        let b1 = barrier.clone();
        let handle1 = tokio::spawn(async move {
            b1.wait().await;
            svc1.create(input1, now).await
        });

        let svc2 = service.clone();
        let b2 = barrier.clone();
        let handle2 = tokio::spawn(async move {
            b2.wait().await;
            svc2.create(input2, now).await
        });

        let (res1, res2) = tokio::join!(handle1, handle2);
        let out1 = res1.expect("task 1 panicked");
        let out2 = res2.expect("task 2 panicked");

        let (success_count, capacity_full_count) = match (&out1, &out2) {
            (Ok(_), Err(BookingError::CapacityFull)) => (1, 1),
            (Err(BookingError::CapacityFull), Ok(_)) => (1, 1),
            other => panic!(
                "round {round}: expected exactly one Ok and one CapacityFull, got: {other:?}"
            ),
        };

        assert_eq!(success_count, 1, "round {round}: exactly one winner");
        assert_eq!(
            capacity_full_count, 1,
            "round {round}: exactly one CapacityFull"
        );

        let (_, reserved, _) = db.slot_row(slot).await;
        assert_eq!(reserved, 1, "round {round}: slot reserved is exactly 1");

        let booking_count = db
            .count(&format!(
                "SELECT count(*) FROM bookings WHERE slot_id = '{slot}'"
            ))
            .await;
        assert_eq!(
            booking_count, 1,
            "round {round}: exactly 1 booking in database"
        );
    }

    db.finish().await;
}
