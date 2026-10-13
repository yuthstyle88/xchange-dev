//! BK-5 #1384 — booking sweeper tests (design §2 B12, §4.3, §5, §9, §9.1).
//!
//! Locks the background sweeper rules:
//! - An overdue PENDING_CUSTOMER_CONFIRMATION -> EXPIRED
//! - A CONFIRMED booking after slot.start_at + grace -> NO_SHOW
//! - Each releases its slot place (booking_slots.reserved decremented atomically)
//! - Every transition writes one booking_events row with actor system
//! - Running twice changes nothing (idempotent)
//! - A booking checked in a moment before the sweep is untouched (compare-and-set)
//! - Policy grace minutes is dynamic from booking_policies, never hard-coded

mod booking_common;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_booking::application::sweeper::{BookingSweeper, SweepCounts};
use xc_booking::domain::booking::booking::Booking;
use xc_booking::domain::booking::booking_machine::BookingMachine;
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::domain::booking::source_channel::SourceChannel;
use xc_booking::domain::policy::booking_policy::BookingPolicy;
use xc_booking::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;

use booking_common::{ORG_1, TestDb};

/// Injected local noon in Asia/Bangkok (12:00 Bangkok = 05:00 UTC).
/// Anchoring `now` at midday eliminates day-boundary flakiness around Bangkok midnight
/// (where `now - 30m` falls on yesterday's service date and triggers `dayEnded` sweep).
fn fixed_noon() -> DateTime<Utc> {
    chrono_tz::Asia::Bangkok
        .with_ymd_and_hms(2026, 10, 15, 12, 0, 0)
        .unwrap()
        .with_timezone(&Utc)
}

fn dummy_slot(start: DateTime<Utc>) -> BookingSlot {
    BookingSlot {
        id: Uuid::new_v4(),
        organization_id: Uuid::nil(),
        branch_id: BranchId::new("MAIN"),
        service_date: NaiveDate::from_ymd_opt(2030, 1, 7).expect("a date"),
        start_at: start,
        end_at: start + Duration::minutes(30),
        capacity: 2,
        reserved: 1,
        status: SlotStatus::Open,
    }
}

fn dummy_booking(status: BookingStatus, expires_at: Option<DateTime<Utc>>) -> Booking {
    Booking {
        id: format!("BK-{}", Uuid::new_v4()),
        organization_id: Uuid::nil(),
        branch_id: BranchId::new("MAIN"),
        booking_no: "B300107-0001".to_string(),
        customer_id: "CX-00000001".to_string(),
        slot_id: Uuid::new_v4(),
        quote_id: None,
        shop_side: Side::Buy,
        requested_currency: "USD".to_string(),
        requested_quantity: Decimal::new(100, 0),
        source_channel: SourceChannel::App,
        status,
        qr_token_hash: [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat(),
        qr_version: 1,
        confirm_expires_at: expires_at,
        checked_in_at: None,
        transaction_id: None,
        partner_attribution_id: None,
        ended_reason: None,
        created_by: "test".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn seed_booking(
    db: &TestDb,
    id: &str,
    org_id: &str,
    branch_id: &str,
    booking_no: &str,
    customer_id: &str,
    slot_id: Uuid,
    source_channel: &str,
    status: &str,
    confirm_expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
) {
    // Unique 32 random bytes satisfying bookings_qr_token_hash_key UNIQUE constraint
    let qr_token_hash = [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    sqlx::query(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, \
         shop_side, requested_currency, requested_quantity, source_channel, status, qr_token_hash, \
         qr_version, confirm_expires_at, created_by, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, 'BUY', 'USD', 100, $7, \
         $8, $9, 1, $10, 'test', $11, $11)",
    )
    .bind(id)
    .bind(Uuid::parse_str(org_id).unwrap())
    .bind(branch_id)
    .bind(booking_no)
    .bind(customer_id)
    .bind(slot_id)
    .bind(source_channel)
    .bind(status)
    .bind(&qr_token_hash)
    .bind(confirm_expires_at)
    .bind(created_at)
    .execute(&db.pool)
    .await
    .expect("seed booking directly");
}

async fn add_slot_rule_in_org(
    db: &TestDb,
    org_id: &str,
    branch: &BranchId,
    slot_minutes: i32,
    capacity: i32,
    horizon_days: i32,
) {
    let user_id = db
        .text(&format!(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ('{org_id}', '{}', 'ACTIVE', 'test') RETURNING id::text",
            booking_common::unique("user")
        ))
        .await
        .expect("user id");
    db.exec(&format!(
        "INSERT INTO booking_slot_rules (organization_id, branch_id, slot_minutes, capacity, \
         horizon_days, updated_by) \
         VALUES ('{org_id}', '{}', {slot_minutes}, {capacity}, {horizon_days}, '{user_id}') \
         ON CONFLICT (branch_id) DO UPDATE SET \
         slot_minutes = EXCLUDED.slot_minutes, \
         capacity = EXCLUDED.capacity, \
         horizon_days = EXCLUDED.horizon_days",
        branch.as_str()
    ))
    .await;
}

// ---------------------------------------------------------------------------
// Pure domain tests on BookingMachine::{expire, no_show}
// ---------------------------------------------------------------------------

#[test]
fn machine_expire_transitions_pending_past_confirm_expires_at_to_expired() {
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let expires_at = now - Duration::minutes(5); // 5 minutes overdue
    let booking = dummy_booking(BookingStatus::PendingCustomerConfirmation, Some(expires_at));

    let result = BookingMachine::expire(&booking, now);
    assert_eq!(
        result.unwrap(),
        BookingStatus::Expired,
        "overdue pending booking transitions to EXPIRED"
    );
}

#[test]
fn machine_expire_refuses_when_before_confirm_expires_at() {
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let expires_at = now + Duration::minutes(15); // not yet expired
    let booking = dummy_booking(BookingStatus::PendingCustomerConfirmation, Some(expires_at));

    let result = BookingMachine::expire(&booking, now);
    assert!(
        result.is_err(),
        "expire before confirm_expires_at must be refused"
    );
}

#[test]
fn machine_expire_refuses_non_pending_status() {
    let now = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let expires_at = now - Duration::minutes(5);

    // Confirmed booking is not pending
    let confirmed_booking = dummy_booking(BookingStatus::Confirmed, Some(expires_at));
    assert!(
        BookingMachine::expire(&confirmed_booking, now).is_err(),
        "only PENDING_CUSTOMER_CONFIRMATION can expire"
    );

    // Already expired booking
    let expired_booking = dummy_booking(BookingStatus::Expired, Some(expires_at));
    assert!(
        BookingMachine::expire(&expired_booking, now).is_err(),
        "already EXPIRED booking cannot expire again"
    );
}

#[test]
fn machine_no_show_transitions_confirmed_past_start_plus_grace_to_no_show() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);

    // now is 20 min after slot start (grace is 15 min, so overdue by 5 min)
    let now = slot_start + Duration::minutes(20);

    let result = BookingMachine::no_show(&booking, now, &policy, &slot);
    assert_eq!(
        result.unwrap(),
        BookingStatus::NoShow,
        "confirmed booking past start + grace transitions to NO_SHOW"
    );
}

#[test]
fn machine_no_show_refuses_when_before_start_plus_grace() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };
    let slot = dummy_slot(slot_start);
    let booking = dummy_booking(BookingStatus::Confirmed, None);

    // Case 1: slot is in the future
    let now_before = slot_start - Duration::minutes(10);
    assert!(
        BookingMachine::no_show(&booking, now_before, &policy, &slot).is_err(),
        "slot in future cannot be no-show"
    );

    // Case 2: slot started 10 min ago, within 15 min grace
    let now_during_grace = slot_start + Duration::minutes(10);
    assert!(
        BookingMachine::no_show(&booking, now_during_grace, &policy, &slot).is_err(),
        "booking within grace period cannot be no-show"
    );

    // Case 3: exactly at boundary (slot_start + 15 min is grace_until)
    let now_exact_grace = slot_start + Duration::minutes(15);
    assert!(
        BookingMachine::no_show(&booking, now_exact_grace, &policy, &slot).is_err(),
        "booking exactly at grace_until is not no-show"
    );
}

#[test]
fn machine_no_show_refuses_non_confirmed_status() {
    let slot_start = Utc.with_ymd_and_hms(2030, 1, 7, 10, 0, 0).unwrap();
    let policy = BookingPolicy {
        cancel_cutoff_minutes: 60,
        no_show_grace_minutes: 15,
        confirm_expiry_minutes: 60,
    };
    let slot = dummy_slot(slot_start);
    let now = slot_start + Duration::minutes(30);

    // Checked-in booking cannot become no-show
    let checked_in_booking = dummy_booking(BookingStatus::CheckedIn, None);
    assert!(
        BookingMachine::no_show(&checked_in_booking, now, &policy, &slot).is_err(),
        "CHECKED_IN booking cannot become NO_SHOW"
    );

    // Cancelled booking cannot become no-show
    let cancelled_booking = dummy_booking(BookingStatus::Cancelled, None);
    assert!(
        BookingMachine::no_show(&cancelled_booking, now, &policy, &slot).is_err(),
        "CANCELLED booking cannot become NO_SHOW"
    );
}

// ---------------------------------------------------------------------------
// Database Sweeper Integration Tests (design §9.1 BK-5 rows, §2 B12, §4.3)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overdue_pending_customer_confirmation_transitions_to_expired_and_releases_place() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("Pending CX", "+66810000001").await;

    let now = fixed_noon();
    let slot_id = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 1)
        .await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    let confirm_expires_at = now - Duration::minutes(5); // 5 min overdue
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261012-0001",
        &customer_id,
        slot_id,
        "APP",
        "PENDING_CUSTOMER_CONFIRMATION",
        Some(confirm_expires_at),
        now - Duration::hours(1),
    )
    .await;

    // Run sweeper
    let sweeper: BookingSweeper = db.sweeper();
    let counts: SweepCounts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(counts.expired, 1, "exactly 1 booking expired");
    assert_eq!(counts.no_show, 0, "no no-show transitions");

    // 1. Assert booking status is EXPIRED
    let status = db
        .text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .expect("booking status");
    assert_eq!(status, "EXPIRED");

    // 2. Assert slot reserved was decremented to 0
    let reserved = db
        .count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot_id}'"
        ))
        .await;
    assert_eq!(reserved, 0, "slot place was released");

    // 3. Assert booking_events row written with actor 'system' (design B12, §4.3)
    let event_count = db
        .count(&format!(
            "SELECT count(*) FROM booking_events WHERE booking_id = '{booking_id}'"
        ))
        .await;
    assert_eq!(event_count, 1, "one event recorded");

    let (kind, from_st, to_st, actor_id): (String, Option<String>, String, String) = sqlx::query_as(
        "SELECT kind, from_status, to_status, actor_id FROM booking_events WHERE booking_id = $1",
    )
    .bind(&booking_id)
    .fetch_one(&db.pool)
    .await
    .expect("event row");

    assert_eq!(kind, "EXPIRE");
    assert_eq!(from_st.as_deref(), Some("PENDING_CUSTOMER_CONFIRMATION"));
    assert_eq!(to_st, "EXPIRED");
    assert_eq!(actor_id, "system", "event actor is system (design B12)");

    db.finish().await;
}

#[tokio::test]
async fn pending_customer_confirmation_before_confirm_expires_at_is_not_expired() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("Pending CX Future", "+66810000002").await;

    let now = fixed_noon();
    let slot_id = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 1)
        .await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    let confirm_expires_at = now + Duration::minutes(20); // 20 min in future
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261012-0002",
        &customer_id,
        slot_id,
        "APP",
        "PENDING_CUSTOMER_CONFIRMATION",
        Some(confirm_expires_at),
        now,
    )
    .await;

    let sweeper = db.sweeper();
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(counts.expired, 0, "not yet expired");
    assert_eq!(counts.no_show, 0);

    let status = db
        .text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .expect("booking status");
    assert_eq!(status, "PENDING_CUSTOMER_CONFIRMATION");

    let reserved = db
        .count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot_id}'"
        ))
        .await;
    assert_eq!(reserved, 1, "reservation retained");

    let event_count = db
        .count(&format!(
            "SELECT count(*) FROM booking_events WHERE booking_id = '{booking_id}'"
        ))
        .await;
    assert_eq!(event_count, 0, "no events written");

    db.finish().await;
}

#[tokio::test]
async fn overdue_confirmed_after_start_plus_grace_transitions_to_no_show_and_releases_place() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    // Policy: grace is 15 minutes
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("NoShow CX", "+66810000003").await;

    let now = fixed_noon();
    // Slot started 20 minutes ago (grace was 15 min, so overdue by 5 min)
    let slot_start = now - Duration::minutes(20);
    let slot_id = db.add_slot(&branch, slot_start, 30, 5, 1).await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261012-0003",
        &customer_id,
        slot_id,
        "APP",
        "CONFIRMED",
        None,
        now - Duration::hours(1),
    )
    .await;

    let sweeper = db.sweeper();
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(counts.expired, 0);
    assert_eq!(counts.no_show, 1, "exactly 1 marked no-show");

    // 1. Booking status is NO_SHOW
    let status = db
        .text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .expect("booking status");
    assert_eq!(status, "NO_SHOW");

    // 2. Slot reserved was decremented to 0
    let reserved = db
        .count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot_id}'"
        ))
        .await;
    assert_eq!(reserved, 0, "slot place was released");

    // 3. Assert booking_events row written with actor 'system'
    let (kind, from_st, to_st, actor_id): (String, Option<String>, String, String) = sqlx::query_as(
        "SELECT kind, from_status, to_status, actor_id FROM booking_events WHERE booking_id = $1",
    )
    .bind(&booking_id)
    .fetch_one(&db.pool)
    .await
    .expect("event row");

    assert_eq!(kind, "NO_SHOW");
    assert_eq!(from_st.as_deref(), Some("CONFIRMED"));
    assert_eq!(to_st, "NO_SHOW");
    assert_eq!(actor_id, "system", "event actor is system (design B12)");

    db.finish().await;
}

#[tokio::test]
async fn confirmed_before_start_plus_grace_is_not_marked_no_show() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("WithinGrace CX", "+66810000004").await;

    let now = fixed_noon();

    // Slot 1: in future (+30 min)
    let slot1 = db
        .add_slot(&branch, now + Duration::minutes(30), 30, 5, 1)
        .await;
    let b1 = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b1,
        ORG_1,
        branch.as_str(),
        "B261012-0004",
        &customer_id,
        slot1,
        "APP",
        "CONFIRMED",
        None,
        now,
    )
    .await;

    // Slot 2: started 5 min ago (15 min grace -> 10 min grace remaining)
    let slot2 = db
        .add_slot(&branch, now - Duration::minutes(5), 30, 5, 1)
        .await;
    let b2 = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b2,
        ORG_1,
        branch.as_str(),
        "B261012-0005",
        &customer_id,
        slot2,
        "APP",
        "CONFIRMED",
        None,
        now - Duration::hours(1),
    )
    .await;

    let sweeper = db.sweeper();
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(counts.expired, 0);
    assert_eq!(counts.no_show, 0, "neither booking is no-show");

    assert_eq!(
        db.text(&format!("SELECT status FROM bookings WHERE id = '{b1}'"))
            .await
            .unwrap(),
        "CONFIRMED"
    );
    assert_eq!(
        db.text(&format!("SELECT status FROM bookings WHERE id = '{b2}'"))
            .await
            .unwrap(),
        "CONFIRMED"
    );

    assert_eq!(
        db.count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot1}'"
        ))
        .await,
        1
    );
    assert_eq!(
        db.count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot2}'"
        ))
        .await,
        1
    );

    db.finish().await;
}

#[tokio::test]
async fn no_show_grace_respects_custom_policy_not_hardcoded_value() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    // Policy: grace is 45 minutes (custom, not the default 15)
    db.add_policy(60, 45, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("CustomGrace CX", "+66810000005").await;

    let now = fixed_noon();
    // Slot started 25 minutes ago:
    // If grace were 15m, this would be NO_SHOW.
    // With 45m grace, 20 minutes remain!
    let slot_start = now - Duration::minutes(25);
    let slot_id = db.add_slot(&branch, slot_start, 30, 5, 1).await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261012-0006",
        &customer_id,
        slot_id,
        "APP",
        "CONFIRMED",
        None,
        now - Duration::hours(1),
    )
    .await;

    let sweeper = db.sweeper();

    // Run at `now`: 25 min after start < 45 min grace -> remains CONFIRMED
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");
    assert_eq!(counts.no_show, 0, "not no-show under 45 min policy");
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .unwrap(),
        "CONFIRMED"
    );

    // Advance clock 25 min (50 min after start > 45 min grace) -> transitions to NO_SHOW
    let later = now + Duration::minutes(25);
    let later_counts = sweeper.run(later).await.expect("sweeper run succeeds");
    assert_eq!(
        later_counts.no_show, 1,
        "transitions to no-show once custom grace passes"
    );

    assert_eq!(
        db.text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .unwrap(),
        "NO_SHOW"
    );

    assert_eq!(
        db.count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot_id}'"
        ))
        .await,
        0,
        "place released"
    );

    db.finish().await;
}

#[tokio::test]
async fn running_sweeper_twice_changes_nothing_idempotent() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("Twice CX", "+66810000006").await;

    let now = fixed_noon();

    // Slot 1: overdue pending booking
    let slot1 = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 1)
        .await;
    let b1 = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b1,
        ORG_1,
        branch.as_str(),
        "B261012-0007",
        &customer_id,
        slot1,
        "APP",
        "PENDING_CUSTOMER_CONFIRMATION",
        Some(now - Duration::minutes(10)),
        now - Duration::hours(1),
    )
    .await;

    // Slot 2: overdue confirmed booking
    let slot2 = db
        .add_slot(&branch, now - Duration::minutes(30), 30, 5, 1)
        .await;
    let b2 = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b2,
        ORG_1,
        branch.as_str(),
        "B261012-0008",
        &customer_id,
        slot2,
        "APP",
        "CONFIRMED",
        None,
        now - Duration::hours(1),
    )
    .await;

    let sweeper = db.sweeper();

    // First run: processes both
    let counts1 = sweeper.run(now).await.expect("first run succeeds");
    assert_eq!(counts1.expired, 1);
    assert_eq!(counts1.no_show, 1);

    assert_eq!(
        db.count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot1}'"
        ))
        .await,
        0
    );
    assert_eq!(
        db.count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot2}'"
        ))
        .await,
        0
    );

    let events_after_run1 = db.count("SELECT count(*) FROM booking_events").await;
    assert_eq!(events_after_run1, 2);

    // Second run immediately after (or 1 minute later): changes NOTHING
    let counts2 = sweeper
        .run(now + Duration::minutes(1))
        .await
        .expect("second run succeeds");
    assert_eq!(counts2.expired, 0, "second run has 0 expired");
    assert_eq!(counts2.no_show, 0, "second run has 0 no-show");

    // Reservations never drop below 0 or decrement a second time
    assert_eq!(
        db.count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot1}'"
        ))
        .await,
        0
    );
    assert_eq!(
        db.count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot2}'"
        ))
        .await,
        0
    );

    // No new events written
    let events_after_run2 = db.count("SELECT count(*) FROM booking_events").await;
    assert_eq!(
        events_after_run2, events_after_run1,
        "running twice writes no new events"
    );

    db.finish().await;
}

#[tokio::test]
async fn booking_checked_in_just_before_sweep_is_untouched_compare_and_set() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("CheckedIn CX", "+66810000007").await;

    let now = fixed_noon();
    // Slot started 30 min ago (past 15m grace)
    let slot_id = db
        .add_slot(&branch, now - Duration::minutes(30), 30, 5, 1)
        .await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    // Booking was checked in at counter before sweep runs
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261012-0009",
        &customer_id,
        slot_id,
        "APP",
        "CHECKED_IN",
        None,
        now - Duration::hours(1),
    )
    .await;

    let sweeper = db.sweeper();
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(
        counts.no_show, 0,
        "checked-in booking is not marked no-show"
    );

    // Booking remains CHECKED_IN
    let status = db
        .text(&format!(
            "SELECT status FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .unwrap();
    assert_eq!(status, "CHECKED_IN");

    // Reservation is still held (CHECKED_IN is a holding state)
    let reserved = db
        .count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot_id}'"
        ))
        .await;
    assert_eq!(reserved, 1, "holding reservation preserved");

    // No events written
    let event_count = db
        .count(&format!(
            "SELECT count(*) FROM booking_events WHERE booking_id = '{booking_id}'"
        ))
        .await;
    assert_eq!(event_count, 0);

    db.finish().await;
}

#[tokio::test]
async fn terminal_and_non_holding_states_are_never_swept() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("Terminal CX", "+66810000008").await;

    let now = fixed_noon();
    let slot_id = db
        .add_slot(&branch, now - Duration::minutes(40), 30, 5, 0)
        .await;

    let b_cancelled = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b_cancelled,
        ORG_1,
        branch.as_str(),
        "B261012-0010",
        &customer_id,
        slot_id,
        "APP",
        "CANCELLED",
        None,
        now - Duration::hours(1),
    )
    .await;

    let b_noshow = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b_noshow,
        ORG_1,
        branch.as_str(),
        "B261012-0011",
        &customer_id,
        slot_id,
        "APP",
        "NO_SHOW",
        None,
        now - Duration::hours(1),
    )
    .await;

    let b_expired = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b_expired,
        ORG_1,
        branch.as_str(),
        "B261012-0012",
        &customer_id,
        slot_id,
        "APP",
        "EXPIRED",
        Some(now - Duration::hours(2)),
        now - Duration::hours(3),
    )
    .await;

    let sweeper = db.sweeper();
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(counts.expired, 0);
    assert_eq!(counts.no_show, 0);

    assert_eq!(
        db.text(&format!(
            "SELECT status FROM bookings WHERE id = '{b_cancelled}'"
        ))
        .await
        .unwrap(),
        "CANCELLED"
    );
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM bookings WHERE id = '{b_noshow}'"
        ))
        .await
        .unwrap(),
        "NO_SHOW"
    );
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM bookings WHERE id = '{b_expired}'"
        ))
        .await
        .unwrap(),
        "EXPIRED"
    );

    db.finish().await;
}

#[tokio::test]
async fn sweeper_handles_multiple_organizations_and_branches() {
    let db = TestDb::start().await;

    // Org 1: 15 min grace
    let branch1 = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch1, 30, 5, 14).await;
    let customer1 = db.add_customer("Org1 CX", "+66810000009").await;

    // Org 2: 60 min grace
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{org2}', 'ORG2', 'Organization 2', 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO booking_policies (organization_id, cancel_cutoff_minutes, no_show_grace_minutes, confirm_expiry_minutes) \
         VALUES ('{org2}', 60, 60, 60)"
    ))
    .await;
    let branch2 = db.add_branch_in_org(org2, "Asia/Bangkok", None, true).await;
    add_slot_rule_in_org(&db, org2, &branch2, 30, 5, 14).await;
    let customer2 = db
        .add_customer_in_org(org2, "Org2 CX", "+66820000010")
        .await;

    let now = fixed_noon();

    // Booking 1: Org 1, slot started 20 min ago -> past 15 min grace -> NO_SHOW
    let slot1 = db
        .add_slot(&branch1, now - Duration::minutes(20), 30, 5, 1)
        .await;
    let b1 = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b1,
        ORG_1,
        branch1.as_str(),
        "B261012-0013",
        &customer1,
        slot1,
        "APP",
        "CONFIRMED",
        None,
        now - Duration::hours(1),
    )
    .await;

    // Booking 2: Org 2, slot started 20 min ago -> within 60 min grace -> remains CONFIRMED
    let slot2 = db
        .add_slot(&branch2, now - Duration::minutes(20), 30, 5, 1)
        .await;
    let b2 = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b2,
        org2,
        branch2.as_str(),
        "B261012-0014",
        &customer2,
        slot2,
        "APP",
        "CONFIRMED",
        None,
        now - Duration::hours(1),
    )
    .await;

    // Booking 3: Org 2, pending confirmation overdue by 5 min -> EXPIRED
    let slot3 = db
        .add_slot(&branch2, now + Duration::hours(1), 30, 5, 1)
        .await;
    let b3 = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &b3,
        org2,
        branch2.as_str(),
        "B261012-0015",
        &customer2,
        slot3,
        "APP",
        "PENDING_CUSTOMER_CONFIRMATION",
        Some(now - Duration::minutes(5)),
        now - Duration::hours(2),
    )
    .await;

    let sweeper = db.sweeper();
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(counts.expired, 1, "b3 expired");
    assert_eq!(counts.no_show, 1, "b1 marked no-show");

    assert_eq!(
        db.text(&format!("SELECT status FROM bookings WHERE id = '{b1}'"))
            .await
            .unwrap(),
        "NO_SHOW"
    );
    assert_eq!(
        db.text(&format!("SELECT status FROM bookings WHERE id = '{b2}'"))
            .await
            .unwrap(),
        "CONFIRMED"
    );
    assert_eq!(
        db.text(&format!("SELECT status FROM bookings WHERE id = '{b3}'"))
            .await
            .unwrap(),
        "EXPIRED"
    );

    db.finish().await;
}

#[tokio::test]
async fn checked_in_past_service_date_with_no_active_ticket_transitions_to_cancelled_day_ended_and_releases_place()
 {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("CheckedIn CX DayEnd", "+66810000099").await;

    let now = fixed_noon();
    // Slot was yesterday (service date in the past)
    let slot_start = now - Duration::days(1);
    let slot_id = db.add_slot(&branch, slot_start, 30, 5, 1).await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    // Booking was CHECKED_IN, checked_in_at yesterday, no active queue ticket
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261011-0001",
        &customer_id,
        slot_id,
        "APP",
        "CHECKED_IN",
        None,
        slot_start - Duration::hours(2),
    )
    .await;

    // Set checked_in_at on the booking
    db.exec(&format!(
        "UPDATE bookings SET checked_in_at = '{}' WHERE id = '{booking_id}'",
        slot_start
    ))
    .await;

    let sweeper = db.sweeper();
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(counts.day_ended, 1, "exactly 1 booking cancelled dayEnded");
    assert_eq!(counts.expired, 0);
    assert_eq!(counts.no_show, 0);

    // 1. Status is CANCELLED, ended_reason is dayEnded
    let (status, ended_reason): (String, Option<String>) =
        sqlx::query_as("SELECT status, ended_reason FROM bookings WHERE id = $1")
            .bind(&booking_id)
            .fetch_one(&db.pool)
            .await
            .expect("booking row");

    assert_eq!(status, "CANCELLED");
    assert_eq!(ended_reason.as_deref(), Some("dayEnded"));

    // 2. Place released (reserved decremented to 0)
    let reserved = db
        .count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot_id}'"
        ))
        .await;
    assert_eq!(reserved, 0, "slot place was released");

    // 3. System CANCEL event written (design B12, §4.3)
    let (kind, from_st, to_st, reason, actor_id): (
        String,
        Option<String>,
        String,
        Option<String>,
        String,
    ) = sqlx::query_as(
        "SELECT kind, from_status, to_status, reason, actor_id FROM booking_events WHERE booking_id = $1",
    )
    .bind(&booking_id)
    .fetch_one(&db.pool)
    .await
    .expect("event row");

    assert_eq!(kind, "CANCEL");
    assert_eq!(from_st.as_deref(), Some("CHECKED_IN"));
    assert_eq!(to_st, "CANCELLED");
    assert_eq!(reason.as_deref(), Some("dayEnded"));
    assert_eq!(actor_id, "system", "event actor is system (design B12)");

    db.finish().await;
}

#[tokio::test]
async fn checked_in_past_service_date_with_active_ticket_is_not_cancelled_by_day_ended_sweep() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("CheckedIn CX Active", "+66810000098").await;

    let now = fixed_noon();
    // Slot was yesterday (service date in the past)
    let slot_start = now - Duration::days(1);
    let slot_id = db.add_slot(&branch, slot_start, 30, 5, 1).await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    // Booking was CHECKED_IN, checked_in_at yesterday, with an active queue ticket
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261011-0002",
        &customer_id,
        slot_id,
        "APP",
        "CHECKED_IN",
        None,
        slot_start - Duration::hours(2),
    )
    .await;

    // Set checked_in_at on the booking
    db.exec(&format!(
        "UPDATE bookings SET checked_in_at = '{}' WHERE id = '{booking_id}'",
        slot_start
    ))
    .await;

    // Customer holds an ACTIVE queue ticket (status WAITING) for this booking
    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, \
         sequence, display_number, status, booking_id, issued_at, idempotency_key) \
         VALUES ($1, $2, $3, (SELECT service_date FROM booking_slots WHERE id = $4), 'BOOKING', 1, 'B001', 'WAITING', $5, $6, $7)",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::parse_str(ORG_1).unwrap())
    .bind(branch.as_str())
    .bind(slot_id)
    .bind(&booking_id)
    .bind(slot_start)
    .bind(Uuid::new_v4().to_string())
    .execute(&db.pool)
    .await
    .expect("seed active queue ticket");

    let sweeper = db.sweeper();
    let counts = sweeper.run(now).await.expect("sweeper run succeeds");

    assert_eq!(
        counts.day_ended, 0,
        "booking with active queue ticket must not be cancelled"
    );
    assert_eq!(counts.expired, 0);
    assert_eq!(counts.no_show, 0);

    // 1. Status remains CHECKED_IN, ended_reason is None
    let (status, ended_reason): (String, Option<String>) =
        sqlx::query_as("SELECT status, ended_reason FROM bookings WHERE id = $1")
            .bind(&booking_id)
            .fetch_one(&db.pool)
            .await
            .expect("booking row");

    assert_eq!(status, "CHECKED_IN");
    assert_eq!(ended_reason, None);

    // 2. Place is not released (reserved remains 1)
    let reserved = db
        .count(&format!(
            "SELECT reserved::bigint FROM booking_slots WHERE id = '{slot_id}'"
        ))
        .await;
    assert_eq!(reserved, 1, "slot place is still reserved");

    // 3. No system CANCEL event written
    let event_count = db
        .count(&format!(
            "SELECT count(*) FROM booking_events WHERE booking_id = '{booking_id}'"
        ))
        .await;
    assert_eq!(
        event_count, 0,
        "no events written for active ticket booking"
    );

    db.finish().await;
}
