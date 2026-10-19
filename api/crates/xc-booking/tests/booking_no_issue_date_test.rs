//! Up-front specification tests for the booking-number date ticket (gap survey defect 2, #564
//! issuecomment-6262974911): `booking_no` and its `booking_sequences.issued_on` are dated by the
//! organisation's local date **of issue**, not by the slot's service date.
//!
//! Design: `docs/design/blueprint/booking-slots.md:172-173` ("`booking_no` comes from
//! `booking_sequences` … `issued_on` is the organisation's local date (`organizations.timezone`)").
//! The number's look `B<yyMMdd>-<0001>` is `BookingNo::format` (`domain/booking/booking_no.rs:9`).
//!
//! Written before the implementation by the ticket author claude/dell-claude-6 (order from the xchange
//! BE controller claude/a90ab16c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod booking_common;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_booking::application::booking_service::CreateBookingInput;
use xc_booking::domain::booking::source_channel::SourceChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;

use booking_common::{ORG_1, TestDb};

fn input(branch: BranchId, slot: Uuid, customer: String) -> CreateBookingInput {
    CreateBookingInput {
        organization_id: Uuid::parse_str(ORG_1).unwrap(),
        branch_id: branch,
        slot_id: slot,
        shop_side: Side::Buy,
        requested_currency: "USD".to_string(),
        requested_quantity: Decimal::new(100, 0),
        channel: SourceChannel::App,
        actor_id: "test-actor".to_string(),
        customer_id: Some(customer),
        contact: None,
        quote_id: None,
        referral_code: None,
        idempotency_key: None,
    }
}

/// A Bangkok organisation and branch with a policy and a rule; answers the branch.
async fn shop(db: &TestDb) -> BranchId {
    db.exec(&format!(
        "UPDATE organizations SET timezone = 'Asia/Bangkok' WHERE id = '{ORG_1}'"
    ))
    .await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    branch
}

async fn issued_on(db: &TestDb, booking_id: &str) -> (String, String) {
    let no = db
        .text(&format!(
            "SELECT booking_no FROM bookings WHERE id = '{booking_id}'"
        ))
        .await
        .expect("the booking row");
    let day = db
        .text(&format!(
            "SELECT issued_on::text FROM booking_sequences WHERE organization_id = '{ORG_1}' \
             ORDER BY issued_on DESC LIMIT 1"
        ))
        .await
        .expect("a sequence row");
    (no, day)
}

fn instant(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
}

/// 1. A booking made on 19 October (Bangkok) for a slot a week later is numbered B261019-0001.
#[tokio::test]
async fn the_number_carries_the_day_it_was_made_not_the_slot_day() {
    let db = TestDb::start().await;
    let branch = shop(&db).await;
    let now = instant(2026, 10, 19, 3, 0); // 10:00 in Bangkok
    let slot = db
        .add_slot(&branch, now + Duration::days(7), 30, 5, 0)
        .await;
    let customer = db.add_customer("App User", "+66811111111").await;

    let created = db
        .service()
        .create(input(branch, slot, customer), now)
        .await
        .expect("create");
    let (no, day) = issued_on(&db, &created.booking.id).await;
    assert_eq!(no, "B261019-0001", "dated by the day of issue");
    assert_eq!(
        day, "2026-10-19",
        "booking_sequences.issued_on is the day of issue"
    );
    db.finish().await;
}

/// 2. The day is the organisation's local day: 18:30 UTC on the 19th is already the 20th in Bangkok.
#[tokio::test]
async fn the_day_is_the_organisations_local_day() {
    let db = TestDb::start().await;
    let branch = shop(&db).await;
    let now = instant(2026, 10, 19, 18, 30); // 01:30 on the 20th in Bangkok
    let slot = db
        .add_slot(&branch, now + Duration::days(3), 30, 5, 0)
        .await;
    let customer = db.add_customer("App User", "+66811111112").await;

    let created = db
        .service()
        .create(input(branch, slot, customer), now)
        .await
        .expect("create");
    let (no, day) = issued_on(&db, &created.booking.id).await;
    assert_eq!(no, "B261020-0001");
    assert_eq!(day, "2026-10-20");
    db.finish().await;
}

/// 3. Two bookings made the same day for slots on different days share one run of numbers.
#[tokio::test]
async fn bookings_made_the_same_day_share_one_sequence() {
    let db = TestDb::start().await;
    let branch = shop(&db).await;
    let now = instant(2026, 10, 19, 3, 0);
    let soon = db
        .add_slot(&branch, now + Duration::days(2), 30, 5, 0)
        .await;
    let later = db
        .add_slot(&branch, now + Duration::days(9), 30, 5, 0)
        .await;
    let a = db.add_customer("A", "+66811111113").await;
    let b = db.add_customer("B", "+66811111114").await;

    let first = db
        .service()
        .create(input(branch.clone(), later, a), now)
        .await
        .expect("first");
    let second = db
        .service()
        .create(input(branch, soon, b), now + Duration::minutes(5))
        .await
        .expect("second");
    assert_eq!(issued_on(&db, &first.booking.id).await.0, "B261019-0001");
    assert_eq!(issued_on(&db, &second.booking.id).await.0, "B261019-0002");
    db.finish().await;
}
