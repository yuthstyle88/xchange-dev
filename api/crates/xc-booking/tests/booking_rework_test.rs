//! BK-3 #1380 rework tests:
//! - CAS stale transitions (transition_in, update_slot_in, reissue_qr_in)
//! - Already-started slot gives slotUnavailable (create and reschedule)
//! - Customer ID from another organization gives notFound
//! - Quote check requires channel BOOKING
//! - Every booking command bumps branch_versions via BranchTx
//! - Teller cancel without reason maps to validationFailed field "reason" on the wire
//! - Subkey derivation in xc-kernel pii/derive.rs survives key rotation

mod booking_common;

use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_booking::application::booking_service::CreateBookingInput;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_booking::domain::booking::booking_event::BookingEventKind;
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::domain::booking::qr_token::QrToken;
use xc_booking::domain::booking::repository::BookingRepository;
use xc_booking::domain::booking::source_channel::SourceChannel;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::money::Side;
use xc_kernel::pii::{PiiKeys, derive_subkey};

use booking_common::{ORG_1, TestDb};

const BANGKOK: &str = "Asia/Bangkok";
const ORG_2: &str = "00000000-0000-0000-0000-000000000002";

async fn ensure_org_2(db: &TestDb) {
    db.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{ORG_2}', 'ORG2', 'Second Org', 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO booking_policies (organization_id, cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes) \
         VALUES ('{ORG_2}', 60, 15, 60) \
         ON CONFLICT (organization_id) DO NOTHING"
    ))
    .await;
}

#[tokio::test]
async fn cas_stale_transitions_are_refused_with_stale_version() {
    let db = TestDb::start().await;
    let branch = db.add_branch(BANGKOK, None, true).await;
    db.add_policy(15, 10, 15).await;
    db.add_slot_rule(&branch, 30, 5, 7).await;

    let now = Utc.with_ymd_and_hms(2030, 6, 1, 8, 0, 0).unwrap();
    let slot_start = now + ChronoDuration::hours(2);
    let slot_id = db.add_slot(&branch, slot_start, 30, 5, 0).await;
    let customer_id = db.add_customer("Test CAS", "+66810000001").await;

    let service = db.service();
    let repo = db.booking_repo();

    // Create a DESK booking (starts CONFIRMED)
    let created = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(customer_id),
                contact: None,
                quote_id: None,
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await
        .expect("create desk booking");

    let booking_id = created.booking.id;

    // 1. transition_in CAS: expected PENDING_CUSTOMER_CONFIRMATION, but row is CONFIRMED -> StaleVersion
    let mut tx = db.pool.begin().await.expect("begin tx");
    let err = repo
        .transition_in(
            &mut tx,
            &booking_id,
            BookingStatus::PendingCustomerConfirmation,
            BookingStatus::Confirmed,
            "teller-1",
            BookingEventKind::Confirm,
            None,
            None,
            now,
        )
        .await;
    assert!(
        matches!(err, Err(BookingError::StaleVersion)),
        "transition_in with wrong expected status returns StaleVersion: {err:?}"
    );
    tx.rollback().await.expect("rollback");

    // 2. update_slot_in CAS: expected PENDING_CUSTOMER_CONFIRMATION, but row is CONFIRMED -> StaleVersion
    let new_slot_id = db
        .add_slot(&branch, slot_start + ChronoDuration::hours(1), 30, 5, 0)
        .await;
    let mut tx = db.pool.begin().await.expect("begin tx");
    let err = repo
        .update_slot_in(
            &mut tx,
            &booking_id,
            BookingStatus::PendingCustomerConfirmation,
            new_slot_id,
            "teller-1",
            now,
        )
        .await;
    assert!(
        matches!(err, Err(BookingError::StaleVersion)),
        "update_slot_in with wrong expected status returns StaleVersion: {err:?}"
    );
    tx.rollback().await.expect("rollback");

    // 3. reissue_qr_in CAS: expected PENDING_CUSTOMER_CONFIRMATION, but row is CONFIRMED -> StaleVersion
    let mut tx = db.pool.begin().await.expect("begin tx");
    let err = repo
        .reissue_qr_in(
            &mut tx,
            &booking_id,
            BookingStatus::PendingCustomerConfirmation,
            &[1u8; 32],
            2,
            "teller-1",
            now,
        )
        .await;
    assert!(
        matches!(err, Err(BookingError::StaleVersion)),
        "reissue_qr_in with wrong expected status returns StaleVersion: {err:?}"
    );
    tx.rollback().await.expect("rollback");

    // 4. Cancel the booking -> status is now CANCELLED
    service
        .cancel(&booking_id, "customer-1", false, None, now)
        .await
        .expect("cancel booking");

    // 5. reissue_qr on CANCELLED booking fails with StaleVersion
    let err = service.reissue_qr(&booking_id, "teller-1", now).await;
    assert!(
        matches!(err, Err(BookingError::StaleVersion)),
        "reissue_qr on CANCELLED booking fails with StaleVersion: {err:?}"
    );

    // 6. Direct CAS check: update_slot_in and reissue_qr_in on CANCELLED booking with expected CONFIRMED
    let mut tx = db.pool.begin().await.expect("begin tx");
    let err_slot = repo
        .update_slot_in(
            &mut tx,
            &booking_id,
            BookingStatus::Confirmed,
            new_slot_id,
            "teller-1",
            now,
        )
        .await;
    assert!(
        matches!(err_slot, Err(BookingError::StaleVersion)),
        "update_slot_in on cancelled booking with expected Confirmed returns StaleVersion: {err_slot:?}"
    );

    let err_qr = repo
        .reissue_qr_in(
            &mut tx,
            &booking_id,
            BookingStatus::Confirmed,
            &[1u8; 32],
            3,
            "teller-1",
            now,
        )
        .await;
    assert!(
        matches!(err_qr, Err(BookingError::StaleVersion)),
        "reissue_qr_in on cancelled booking with expected Confirmed returns StaleVersion: {err_qr:?}"
    );
    tx.rollback().await.expect("rollback");

    db.finish().await;
}

#[tokio::test]
async fn already_started_slot_gives_slot_unavailable() {
    let db = TestDb::start().await;
    let branch = db.add_branch(BANGKOK, None, true).await;
    db.add_policy(15, 10, 15).await;
    db.add_slot_rule(&branch, 30, 5, 7).await;

    let now = Utc.with_ymd_and_hms(2030, 6, 1, 10, 0, 0).unwrap();
    // Slot that started 5 minutes ago
    let past_slot_start = now - ChronoDuration::minutes(5);
    let past_slot_id = db.add_slot(&branch, past_slot_start, 30, 5, 0).await;

    // Slot that starts exactly at now
    let exact_now_slot_id = db.add_slot(&branch, now, 30, 5, 0).await;

    let customer_id = db.add_customer("Past Booker", "+66810000002").await;
    let service = db.service();

    // 1. Attempt create on slot starting 5 minutes before now
    let err = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id: past_slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(customer_id.clone()),
                contact: None,
                quote_id: None,
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await;
    assert!(
        matches!(err, Err(BookingError::SlotUnavailable)),
        "create for past slot returns SlotUnavailable: {err:?}"
    );

    // 2. Attempt create on slot starting exactly at now (start_at <= now)
    let err2 = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id: exact_now_slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(customer_id.clone()),
                contact: None,
                quote_id: None,
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await;
    assert!(
        matches!(err2, Err(BookingError::SlotUnavailable)),
        "create for slot at start instant returns SlotUnavailable: {err2:?}"
    );

    // 3. Create a valid future booking, then attempt reschedule to past slot
    let future_slot_start = now + ChronoDuration::hours(1);
    let future_slot_id = db.add_slot(&branch, future_slot_start, 30, 5, 0).await;
    let created = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id: future_slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(customer_id),
                contact: None,
                quote_id: None,
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await
        .expect("create future booking");

    let err_resched = service
        .reschedule(&created.booking.id, past_slot_id, "teller-1", true, now)
        .await;
    assert!(
        matches!(err_resched, Err(BookingError::SlotUnavailable)),
        "reschedule to past slot returns SlotUnavailable: {err_resched:?}"
    );

    db.finish().await;
}

#[tokio::test]
async fn customer_id_from_another_org_gives_not_found() {
    let db = TestDb::start().await;
    ensure_org_2(&db).await;

    let branch = db.add_branch(BANGKOK, None, true).await;
    db.add_policy(15, 10, 15).await;
    db.add_slot_rule(&branch, 30, 5, 7).await;

    let now = Utc.with_ymd_and_hms(2030, 6, 1, 10, 0, 0).unwrap();
    let slot_id = db
        .add_slot(&branch, now + ChronoDuration::hours(2), 30, 5, 0)
        .await;

    // Customer belonging to ORG_2
    let org2_customer_id = db
        .add_customer_in_org(ORG_2, "Foreign Customer", "+66899999999")
        .await;

    let service = db.service();
    let err = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(org2_customer_id.clone()),
                contact: None,
                quote_id: None,
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await;

    assert!(
        matches!(err, Err(BookingError::NotFound)),
        "customer from foreign org returns NotFound: {err:?}"
    );

    // Assert no booking row written
    let count = db.count("SELECT count(*) FROM bookings").await;
    assert_eq!(count, 0, "no booking row written");

    db.finish().await;
}

#[tokio::test]
async fn quote_check_requires_channel_booking() {
    let db = TestDb::start().await;
    let branch = db.add_branch(BANGKOK, None, true).await;
    db.add_policy(15, 10, 15).await;
    db.add_slot_rule(&branch, 30, 5, 7).await;

    let now = Utc.with_ymd_and_hms(2030, 6, 1, 10, 0, 0).unwrap();
    let slot_id = db
        .add_slot(&branch, now + ChronoDuration::hours(2), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("Quote Customer", "+66810000003").await;

    // Insert quote with channel DESK (valid currency USD, qty 100)
    let desk_quote_id = db
        .add_quote(&branch, Side::Buy, "USD", Decimal::from(100), "ACTIVE")
        .await;
    db.exec(&format!(
        "UPDATE quotes SET channel = 'DESK' WHERE id = '{desk_quote_id}'"
    ))
    .await;

    let service = db.service();

    // Channel DESK fails
    let err = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(customer_id.clone()),
                contact: None,
                quote_id: Some(desk_quote_id),
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await;

    assert!(
        matches!(
            err,
            Err(BookingError::ValidationFailed { field: "quoteId" })
        ),
        "DESK quote fails validation on quoteId: {err:?}"
    );

    // Channel APP fails
    db.exec(&format!(
        "UPDATE quotes SET channel = 'APP' WHERE id = '{desk_quote_id}'"
    ))
    .await;

    let err2 = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(customer_id.clone()),
                contact: None,
                quote_id: Some(desk_quote_id),
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await;

    assert!(
        matches!(
            err2,
            Err(BookingError::ValidationFailed { field: "quoteId" })
        ),
        "APP quote fails validation on quoteId: {err2:?}"
    );

    // Channel BOOKING succeeds
    db.exec(&format!(
        "UPDATE quotes SET channel = 'BOOKING' WHERE id = '{desk_quote_id}'"
    ))
    .await;

    let ok = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(customer_id),
                contact: None,
                quote_id: Some(desk_quote_id),
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await;

    assert!(ok.is_ok(), "BOOKING quote succeeds: {ok:?}");

    db.finish().await;
}

#[tokio::test]
async fn every_booking_command_bumps_branch_versions_via_branch_tx() {
    let db = TestDb::start().await;
    let branch = db.add_branch(BANGKOK, None, true).await;
    db.add_policy(15, 10, 15).await;
    db.add_slot_rule(&branch, 30, 5, 7).await;

    let now = Utc.with_ymd_and_hms(2030, 6, 1, 10, 0, 0).unwrap();
    let slot_id = db
        .add_slot(&branch, now + ChronoDuration::hours(2), 30, 5, 0)
        .await;
    let new_slot_id = db
        .add_slot(&branch, now + ChronoDuration::hours(3), 30, 5, 0)
        .await;
    let customer_id = db.add_customer("Version Customer", "+66810000004").await;

    let transactor = pos_infra::transactions::Transactor::new(db.pool.clone());
    let branch_tx = BranchTx::new(transactor);
    let initial_version = branch_tx.version(&branch).await.expect("version");

    let service = db.service();

    // 1. Create (DESK channel starts CONFIRMED)
    let created = service
        .create(
            CreateBookingInput {
                organization_id: Uuid::parse_str(ORG_1).unwrap(),
                branch_id: branch.clone(),
                slot_id,
                shop_side: Side::Buy,
                requested_currency: "USD".to_string(),
                requested_quantity: Decimal::from(100),
                channel: SourceChannel::Desk,
                actor_id: "teller-1".to_string(),
                customer_id: Some(customer_id.clone()),
                contact: None,
                quote_id: None,
                referral_code: None,
                idempotency_key: None,
            },
            now,
        )
        .await
        .expect("create");

    let v1 = branch_tx.version(&branch).await.expect("version");
    assert_eq!(v1, initial_version + 1, "create bumps branch version");

    let booking_id = created.booking.id;

    // 2. Reissue QR
    service
        .reissue_qr(&booking_id, "teller-1", now + ChronoDuration::minutes(6))
        .await
        .expect("reissue_qr");

    let v2 = branch_tx.version(&branch).await.expect("version");
    assert_eq!(v2, v1 + 1, "reissue_qr bumps branch version");

    // 3. Reschedule
    service
        .reschedule(
            &booking_id,
            new_slot_id,
            "customer-1",
            false,
            now + ChronoDuration::minutes(7),
        )
        .await
        .expect("reschedule");

    let v3 = branch_tx.version(&branch).await.expect("version");
    assert_eq!(v3, v2 + 1, "reschedule bumps branch version");

    // 4. Cancel
    service
        .cancel(
            &booking_id,
            "teller-1",
            true,
            Some("Client request"),
            now + ChronoDuration::minutes(8),
        )
        .await
        .expect("cancel");

    let v4 = branch_tx.version(&branch).await.expect("version");
    assert_eq!(v4, v3 + 1, "cancel bumps branch version");

    // 5. Confirm on a PENDING_CUSTOMER_CONFIRMATION booking
    let pending_booking_id = "BK-pending-version-test";
    let pending_slot_id = db
        .add_slot(&branch, now + ChronoDuration::hours(4), 30, 5, 0)
        .await;
    let pending_partner_attr = Uuid::new_v4();
    let qr_token_hash = [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    sqlx::query(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, \
         shop_side, requested_currency, requested_quantity, source_channel, partner_attribution_id, \
         status, qr_token_hash, qr_version, confirm_expires_at, created_by, created_at, updated_at) \
         VALUES ($1, $2, $3, 'B300601-9999', $4, $5, 'BUY', 'USD', 100, 'PARTNER', $6, \
         'PENDING_CUSTOMER_CONFIRMATION', $7, 1, $8, 'test', $9, $9)",
    )
    .bind(pending_booking_id)
    .bind(Uuid::parse_str(ORG_1).unwrap())
    .bind(branch.as_str())
    .bind(&customer_id)
    .bind(pending_slot_id)
    .bind(pending_partner_attr)
    .bind(&qr_token_hash)
    .bind(now + ChronoDuration::hours(1))
    .bind(now)
    .execute(&db.pool)
    .await
    .expect("seed pending");

    let v_before_confirm = branch_tx.version(&branch).await.expect("version");
    service
        .confirm(
            pending_booking_id,
            &customer_id,
            now + ChronoDuration::minutes(9),
        )
        .await
        .expect("confirm");
    let v_after_confirm = branch_tx.version(&branch).await.expect("version");
    assert_eq!(
        v_after_confirm,
        v_before_confirm + 1,
        "confirm bumps branch version"
    );

    db.finish().await;
}

#[test]
fn teller_cancel_without_reason_maps_to_validation_failed_reason_on_wire() {
    let err = BookingError::TellerCancelReasonRequired;
    assert_eq!(err.code(), "tellerCancelReasonRequired");
    assert_eq!(err.wire_code(), "validationFailed");
    assert_eq!(err.wire_field(), Some("reason"));

    let app_err: pos_error::AppError = err.into();
    match app_err {
        pos_error::AppError::Validation(field) => {
            assert_eq!(field, "reason");
        }
        other => panic!("expected AppError::Validation(\"reason\"), got: {other:?}"),
    }
}

#[test]
fn subkey_derivation_deterministic_across_key_rotation() {
    let pii_keys_v1 =
        PiiKeys::parse("k1:2iq82iq82iq82iq82iq82iq82iq82iq82iq82iq82io=").expect("parse");
    let pii_keys_v2 = PiiKeys::parse("k1:2iq82iq82iq82iq82iq82iq82iq82iq82iq82iq82io=\nk2:65+/65+/65+/65+/65+/65+/65+/65+/65+/65+/658=").expect("parse");

    let subkey1 = derive_subkey(&pii_keys_v1, b"xc-booking-qr-v1");
    let subkey2 = derive_subkey(&pii_keys_v2, b"xc-booking-qr-v1");

    assert_eq!(
        subkey1, subkey2,
        "subkey derived from oldest key survives key rotation"
    );

    let qr_key = QrToken::key_from_pii(&pii_keys_v2);
    assert_eq!(qr_key, subkey1);
}
