//! #1937 — `BookingSweeper::run` must raise `branch_versions` for every branch
//! whose booking transitions (`EXPIRED`, `NO_SHOW`, or `CANCELLED` with `dayEnded`).

mod booking_common;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_infra::transactions::Transactor;
use uuid::Uuid;
use xc_booking::application::sweeper::{BookingSweeper, SweepCounts};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use booking_common::{ORG_1, TestDb};

/// Injected local noon in Asia/Bangkok (12:00 Bangkok = 05:00 UTC).
fn fixed_noon() -> DateTime<Utc> {
    chrono_tz::Asia::Bangkok
        .with_ymd_and_hms(2026, 10, 15, 12, 0, 0)
        .unwrap()
        .with_timezone(&Utc)
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

async fn branch_version(db: &TestDb, branch: &BranchId) -> i64 {
    let branch_tx = BranchTx::new(Transactor::new(db.pool.clone()));
    branch_tx.version(branch).await.expect("branch version")
}

// ---------------------------------------------------------------------------
// §9 Row 1: Overdue PENDING_CUSTOMER_CONFIRMATION swept to EXPIRED -> B's version rises
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overdue_pending_customer_confirmation_swept_to_expired_raises_branch_version() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("Expire Version CX", "+66810000101").await;

    let now = fixed_noon();
    let slot_id = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 1)
        .await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261015-0101",
        &customer_id,
        slot_id,
        "APP",
        "PENDING_CUSTOMER_CONFIRMATION",
        Some(now - Duration::minutes(5)),
        now - Duration::hours(1),
    )
    .await;

    let v_before = branch_version(&db, &branch).await;

    let sweeper: BookingSweeper = db.sweeper();
    let counts: SweepCounts = sweeper.run(now).await.expect("sweeper run succeeds");
    assert_eq!(counts.expired, 1, "one booking expired");

    let v_after = branch_version(&db, &branch).await;
    assert!(
        v_after > v_before,
        "sweeping overdue PENDING_CUSTOMER_CONFIRMATION to EXPIRED must raise branch_versions \
         (v_before={v_before}, v_after={v_after})"
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// §9 Row 2: Overdue CONFIRMED swept to NO_SHOW -> B's version rises
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overdue_confirmed_swept_to_no_show_raises_branch_version() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("NoShow Version CX", "+66810000102").await;

    let now = fixed_noon();
    // Slot started 20 minutes ago (past 15-minute grace)
    let slot_id = db
        .add_slot(&branch, now - Duration::minutes(20), 30, 5, 1)
        .await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261015-0102",
        &customer_id,
        slot_id,
        "APP",
        "CONFIRMED",
        None,
        now - Duration::hours(1),
    )
    .await;

    let v_before = branch_version(&db, &branch).await;

    let sweeper: BookingSweeper = db.sweeper();
    let counts: SweepCounts = sweeper.run(now).await.expect("sweeper run succeeds");
    assert_eq!(counts.no_show, 1, "one booking marked NO_SHOW");

    let v_after = branch_version(&db, &branch).await;
    assert!(
        v_after > v_before,
        "sweeping overdue CONFIRMED to NO_SHOW must raise branch_versions \
         (v_before={v_before}, v_after={v_after})"
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// §9 Row 3: dayEnded cancellation by the sweeper -> B's version rises
// ---------------------------------------------------------------------------

#[tokio::test]
async fn checked_in_past_service_date_swept_to_day_ended_raises_branch_version() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("DayEnded Version CX", "+66810000103").await;

    let now = fixed_noon();
    let slot_start = now - Duration::days(1);
    let slot_id = db.add_slot(&branch, slot_start, 30, 5, 1).await;

    let booking_id = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &booking_id,
        ORG_1,
        branch.as_str(),
        "B261014-0103",
        &customer_id,
        slot_id,
        "APP",
        "CHECKED_IN",
        None,
        slot_start - Duration::hours(2),
    )
    .await;
    db.exec(&format!(
        "UPDATE bookings SET checked_in_at = '{}' WHERE id = '{booking_id}'",
        slot_start
    ))
    .await;

    let v_before = branch_version(&db, &branch).await;

    let sweeper: BookingSweeper = db.sweeper();
    let counts: SweepCounts = sweeper.run(now).await.expect("sweeper run succeeds");
    assert_eq!(counts.day_ended, 1, "one booking cancelled with dayEnded");

    let v_after = branch_version(&db, &branch).await;
    assert!(
        v_after > v_before,
        "sweeping CHECKED_IN booking to CANCELLED (dayEnded) must raise branch_versions \
         (v_before={v_before}, v_after={v_after})"
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// §9 Row 4: Nothing overdue (a booking before its deadline) -> B's version is unchanged
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nothing_overdue_leaves_branch_version_unchanged() {
    let db = TestDb::start().await;
    let branch = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch, 30, 5, 14).await;
    let customer_id = db.add_customer("NotOverdue CX", "+66810000104").await;

    let now = fixed_noon();
    let slot_id = db
        .add_slot(&branch, now + Duration::hours(2), 30, 5, 2)
        .await;

    let pending_id = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &pending_id,
        ORG_1,
        branch.as_str(),
        "B261015-0104",
        &customer_id,
        slot_id,
        "APP",
        "PENDING_CUSTOMER_CONFIRMATION",
        Some(now + Duration::minutes(30)),
        now - Duration::minutes(10),
    )
    .await;

    let confirmed_id = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &confirmed_id,
        ORG_1,
        branch.as_str(),
        "B261015-0105",
        &customer_id,
        slot_id,
        "APP",
        "CONFIRMED",
        None,
        now - Duration::minutes(10),
    )
    .await;

    let v_before = branch_version(&db, &branch).await;

    let sweeper: BookingSweeper = db.sweeper();
    let counts: SweepCounts = sweeper.run(now).await.expect("sweeper run succeeds");
    assert_eq!(counts.expired, 0);
    assert_eq!(counts.no_show, 0);
    assert_eq!(counts.day_ended, 0);

    let v_after = branch_version(&db, &branch).await;
    assert_eq!(
        v_after, v_before,
        "when nothing is overdue, branch_versions must remain unchanged"
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// §9 Row 5: Two branches, only branch A's booking overdue -> A rises, B is unchanged
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_branches_only_overdue_branch_version_rises() {
    let db = TestDb::start().await;
    let branch_a = db.add_branch("Asia/Bangkok", None, true).await;
    let branch_b = db.add_branch("Asia/Bangkok", None, true).await;
    db.add_policy(60, 15, 60).await;
    db.add_slot_rule(&branch_a, 30, 5, 14).await;
    db.add_slot_rule(&branch_b, 30, 5, 14).await;
    let customer_a = db.add_customer("BranchA CX", "+66810000105").await;
    let customer_b = db.add_customer("BranchB CX", "+66810000106").await;

    let now = fixed_noon();

    // Branch A: overdue PENDING_CUSTOMER_CONFIRMATION
    let slot_a = db
        .add_slot(&branch_a, now + Duration::hours(2), 30, 5, 1)
        .await;
    let booking_a = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &booking_a,
        ORG_1,
        branch_a.as_str(),
        "B261015-0106",
        &customer_a,
        slot_a,
        "APP",
        "PENDING_CUSTOMER_CONFIRMATION",
        Some(now - Duration::minutes(10)),
        now - Duration::hours(1),
    )
    .await;

    // Branch B: not overdue PENDING_CUSTOMER_CONFIRMATION
    let slot_b = db
        .add_slot(&branch_b, now + Duration::hours(2), 30, 5, 1)
        .await;
    let booking_b = format!("BK-{}", Uuid::new_v4());
    seed_booking(
        &db,
        &booking_b,
        ORG_1,
        branch_b.as_str(),
        "B261015-0107",
        &customer_b,
        slot_b,
        "APP",
        "PENDING_CUSTOMER_CONFIRMATION",
        Some(now + Duration::minutes(30)),
        now - Duration::minutes(10),
    )
    .await;

    let va_before = branch_version(&db, &branch_a).await;
    let vb_before = branch_version(&db, &branch_b).await;

    let sweeper: BookingSweeper = db.sweeper();
    let counts: SweepCounts = sweeper.run(now).await.expect("sweeper run succeeds");
    assert_eq!(counts.expired, 1);

    let va_after = branch_version(&db, &branch_a).await;
    let vb_after = branch_version(&db, &branch_b).await;

    assert!(
        va_after > va_before,
        "overdue branch A must have its branch_versions raised (before={va_before}, after={va_after})"
    );
    assert_eq!(
        vb_after, vb_before,
        "non-overdue branch B must keep its branch_versions unchanged"
    );

    db.finish().await;
}
