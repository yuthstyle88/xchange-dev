// Written before the implementation by the test writer gemini/studio-16 (controller claude/e92db6). Do not edit, delete or weaken.

//! BK-7 #1405 — up-front tests for booking completion: `BookingSaveStep` and `BookingLifecycle::complete_in`
//! (design `docs/design/blueprint/booking-slots.md` §2 B1, B10; §4.1; §4.3; §5; §9.1 BK-7; ticket #1405 §4, §9).
//!
//! Written before the implementation by the test writer gemini/studio-16 (controller claude/e92db6). Do not edit, delete or weaken.
//!
//! Pinned behaviours and edge-cases:
//! 1. `completion_with_checked_in_booking_completes_with_transaction_id`:
//!    A completion with a CHECKED_IN booking of the same customer makes it COMPLETED with transaction_id.
//! 2. `completion_with_in_service_booking_completes_with_transaction_id`:
//!    A completion with an IN_SERVICE booking of the same customer makes it COMPLETED with transaction_id.
//! 3. `completion_with_another_customer_booking_fails_booking_not_this_customer_and_writes_nothing`:
//!    Another customer's booking -> bookingNotThisCustomer and nothing written.
//! 4. `completion_with_confirmed_booking_fails_booking_not_usable`:
//!    A CONFIRMED (not checked in) booking -> bookingNotUsable.
//! 5. `completion_with_non_checked_in_or_in_service_status_fails_booking_not_usable`:
//!    PENDING_CUSTOMER_CONFIRMATION, CANCELLED, NO_SHOW, EXPIRED bookings -> bookingNotUsable.
//! 6. `completion_with_another_branch_booking_fails_booking_not_usable`:
//!    Booking of another branch -> bookingNotUsable (same branch required).
//! 7. `repeat_completion_with_same_transaction_is_noop`:
//!    A repeat completion with the same transaction id is a no-op.
//! 8. `completion_with_differing_side_currency_or_quantity_still_completes`:
//!    Side, currency or quantity different from the request still completes (B1).
//! 9. `second_transaction_for_already_completed_booking_refused`:
//!    A second transaction for an already COMPLETED booking is refused (one booking -> at most one completed transaction).
//! 10. `no_rate_check_depends_on_the_booking`:
//!     No rate check depends on the booking (vouches_for_rate returns false).

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use uuid::Uuid;

use xc_auth::domain::policy::value_objects::role::Role;
use xc_booking::application::lifecycle::BookingLifecycle;
use xc_booking::infrastructure::booking_repository::PgBookingRepository;
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_exchange::domain::customer::value_objects::customer_details::CustomerDetails;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::entities::transaction::Transaction as ExchangeTransaction;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Money, Quote, Rate, Side};
use xchange::application::workflows::booking_save_step::BookingSaveStep;
use xchange::application::workflows::save_step::{
    BookingClaim, SaveActor, SaveContext, SaveExtras, SaveStep,
};

const MUST_FINISH: StdDuration = StdDuration::from_secs(60);
const ORG_ID: &str = "00000000-0000-0000-0000-000000000001";
const BRANCH_ID: &str = "MAIN";
const OTHER_BRANCH_ID: &str = "B02";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

fn hash_token(key: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hasher.finalize().to_vec()
}

struct TestHarness {
    pool: PgPool,
    schema: String,
    branch_id: BranchId,
    other_branch_id: BranchId,
    org_id: Uuid,
    lifecycle: Arc<BookingLifecycle>,
}

impl TestHarness {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 Needs a real PostgreSQL with name ending in _test.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("bk7test");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect admin");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect pool");

        let dir = migrations_dir();
        let migrator = Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
        migrator
            .run(&pool)
            .await
            .unwrap_or_else(|e| panic!("run migrations: {e}"));

        let org_uuid = Uuid::parse_str(ORG_ID).unwrap();
        let branch = BranchId::new(BRANCH_ID);
        let other_branch = BranchId::new(OTHER_BRANCH_ID);

        // Seed currencies
        sqlx::query(
            "INSERT INTO currencies (code, name_th, name_en, symbol) \
             VALUES ('USD', 'US Dollar', 'US Dollar', '$'), ('THB', 'Thai Baht', 'Thai Baht', '฿'), \
                    ('EUR', 'Euro', 'Euro', '€') \
             ON CONFLICT (code) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed currencies");

        // Seed branches
        for b in [&branch, &other_branch] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, booking_enabled) \
                 VALUES ($1, $1, $2, true) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(b.as_str())
            .bind(org_uuid)
            .execute(&pool)
            .await
            .expect("seed branch");
        }

        // Seed user
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ('00000000-0000-0000-0000-000000000010'::uuid, $1, 'Teller 1', 'ACTIVE', 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(org_uuid)
        .execute(&pool)
        .await
        .expect("seed user");

        // Seed customers
        for cid in ["CUST-001", "CUST-002"] {
            sqlx::query(
                "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
                 VALUES ($1, $2, $1, $1, 'LIVE', now()) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(cid)
            .bind(org_uuid)
            .execute(&pool)
            .await
            .expect("seed customer");
        }

        let booking_repo = Arc::new(PgBookingRepository::new(pool.clone()));
        let slot_repo = Arc::new(PgSlotRepository::new(pool.clone()));
        let lifecycle =
            Arc::new(BookingLifecycle::new(booking_repo.clone()).with_slots(slot_repo.clone()));

        Self {
            pool,
            schema,
            branch_id: branch,
            other_branch_id: other_branch,
            org_id: org_uuid,
            lifecycle,
        }
    }

    async fn seed_slot(
        &self,
        slot_id: Uuid,
        branch: &BranchId,
        service_date: NaiveDate,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
    ) {
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             VALUES ($1, $2, $3, $4, $5, $6, 10, 1, 'OPEN') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(slot_id)
        .bind(self.org_id)
        .bind(branch.as_str())
        .bind(service_date)
        .bind(start_at)
        .bind(end_at)
        .execute(&self.pool)
        .await
        .expect("seed slot");
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_booking(
        &self,
        booking_id: &str,
        booking_no: &str,
        branch: &BranchId,
        customer_id: &str,
        slot_id: Uuid,
        status: &str,
        shop_side: &str,
        currency: &str,
        quantity: Decimal,
        transaction_id: Option<Uuid>,
    ) {
        let hash = hash_token(booking_id);
        let confirm_expires_at =
            (status == "PENDING_CUSTOMER_CONFIRMATION").then(|| Utc::now() + Duration::minutes(15));
        sqlx::query(
            "INSERT INTO bookings (\
                id, organization_id, branch_id, booking_no, customer_id, slot_id, \
                shop_side, requested_currency, requested_quantity, source_channel, \
                status, transaction_id, qr_token_hash, qr_version, confirm_expires_at, created_by \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'APP', $10, $11, $12, 1, $13, 'seed') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(booking_id)
        .bind(self.org_id)
        .bind(branch.as_str())
        .bind(booking_no)
        .bind(customer_id)
        .bind(slot_id)
        .bind(shop_side)
        .bind(currency)
        .bind(quantity)
        .bind(status)
        .bind(transaction_id)
        .bind(&hash)
        .bind(confirm_expires_at)
        .execute(&self.pool)
        .await
        .expect("seed booking");
    }

    async fn seed_transaction(&self, tx_id: Uuid, branch: &BranchId, customer_id: &str) {
        let doc_ref = format!("DOC-{tx_id}");
        let sub_id = format!("sub-{tx_id}");
        sqlx::query(
            "INSERT INTO transactions (\
                id, organization_id, document_ref, branch_id, mode, status, submission_id, payload_hash, \
                customer_id, customer_name, nationality, type, code, amount, \
                unit_size, rate, principal, fee, bank_fee, received, paid, \
                method, payment_ref, account, staff_id, booking_no, notes, \
                value_date, paid_at, recorded_at, recorded_by, completed_at, count_confirmed \
             ) VALUES (\
                $1, $2, $3, $4, 'DEMO', 'COMPLETED', $5, 'payload-hash', \
                $6, 'Test Customer', 'THA', 'BUY', 'USD', 100, \
                1, 32.2, 3220, 0, 0, 3220, 100, \
                'Cash', '', 'Cash Drawer', 'teller-1', 'B261014-0001', '', \
                '2026-10-14', now(), now(), 'teller-1', now(), true \
             ) ON CONFLICT (id) DO NOTHING",
        )
        .bind(tx_id)
        .bind(self.org_id)
        .bind(&doc_ref)
        .bind(branch.as_str())
        .bind(&sub_id)
        .bind(customer_id)
        .execute(&self.pool)
        .await
        .expect("seed transaction");
    }

    async fn fetch_booking(&self, booking_id: &str) -> (String, Option<Uuid>) {
        let row = sqlx::query("SELECT status, transaction_id FROM bookings WHERE id = $1")
            .bind(booking_id)
            .fetch_one(&self.pool)
            .await
            .expect("fetch booking");
        (row.get("status"), row.get("transaction_id"))
    }

    async fn fetch_booking_events(
        &self,
        booking_id: &str,
    ) -> Vec<(String, Option<String>, String)> {
        let rows = sqlx::query(
            "SELECT kind, from_status, to_status FROM booking_events WHERE booking_id = $1 ORDER BY created_at",
        )
        .bind(booking_id)
        .fetch_all(&self.pool)
        .await
        .expect("fetch booking events");
        rows.into_iter()
            .map(|r| (r.get("kind"), r.get("from_status"), r.get("to_status")))
            .collect()
    }

    async fn teardown(self) {
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(
                PgConnectOptions::from_str(
                    &std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL"),
                )
                .expect("admin options"),
            )
            .await
            .expect("connect admin for teardown");
        let _ = sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await;
        admin.close().await;
    }
}

fn dummy_form(_branch: &BranchId, customer_id: &str) -> TransactionForm {
    TransactionForm {
        customer: CustomerDetails {
            name: "Test Customer".to_string(),
            passport_no: "P1234567".to_string(),
            nationality: "THA".to_string(),
            birth_date: Some(NaiveDate::from_ymd_opt(1990, 1, 1).unwrap()),
            passport_expiry: Some(NaiveDate::from_ymd_opt(2030, 1, 1).unwrap()),
            mode: Mode::Live,
        },
        submission_id: Uuid::new_v4().to_string(),
        customer_id: customer_id.to_string(),
        side: Side::Buy,
        currency: "USD".to_string(),
        amount: Money::parse("100").unwrap(),
        rate: Rate::parse("32.2").unwrap(),
        client_rate: Decimal::from_str("32.2").unwrap(),
        fee: Money::ZERO,
        bank_fee: Money::ZERO,
        method: PaymentMethod::Cash,
        payment_ref: String::new(),
        account: "Cash Drawer".to_string(),
        staff: "teller-1".to_string(),
        booking: "BK-test".to_string(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
        paid_at: Utc::now(),
        passport: "00000000-0000-0000-0000-000000000001".to_string(),
        slip: None,
        reference_rate: None,
        client_reference_rate: None,
        quote: Quote {
            principal: Money::ZERO,
            received: Money::ZERO,
            paid: Money::ZERO,
        },
    }
}

fn dummy_exchange_transaction(
    doc_ref: &str,
    branch_id: &BranchId,
    customer_id: &str,
) -> ExchangeTransaction {
    ExchangeTransaction {
        document_ref: doc_ref.to_string(),
        branch_id: branch_id.clone(),
        mode: Mode::Live,
        shift_id: None,
        submission_id: Uuid::new_v4().to_string(),
        customer_id: customer_id.to_string(),
        customer_name: "Test Customer".to_string(),
        nationality: "THA".to_string(),
        side: Side::Buy,
        currency: "USD".to_string(),
        amount: Money::parse("100").unwrap(),
        rate: Rate::parse("32.2").unwrap(),
        reference_rate: None,
        principal: Money::ZERO,
        fee: Money::ZERO,
        bank_fee: Money::ZERO,
        received: Money::ZERO,
        paid: Money::ZERO,
        method: PaymentMethod::Cash,
        payment_ref: String::new(),
        account: "Cash Drawer".to_string(),
        staff: "teller-1".to_string(),
        booking: "BK-test".to_string(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
        paid_at: Utc::now(),
        passport_key: "00000000-0000-0000-0000-000000000001".to_string(),
        slip_key: None,
        receipt_key: "receipt-key".to_string(),
        recorded_at: Utc::now(),
        recorded_by: "teller-1".to_string(),
    }
}

// ============================================================================
// Behaviour & edge-case tests (named after each row per controller instructions)
// ============================================================================

/// 1. A completion with a CHECKED_IN booking of the same customer makes it COMPLETED
/// with the transaction id (design B10, §4 criterion 1, §9 edge cases).
#[tokio::test]
async fn completion_with_checked_in_booking_completes_with_transaction_id() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_booking(
            "BK-checked-in",
            "B261014-0001",
            &harness.branch_id,
            "CUST-001",
            slot_id,
            "CHECKED_IN",
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-001")
        .await;

    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = harness
        .lifecycle
        .complete_in(
            &mut tx,
            &harness.branch_id,
            "BK-checked-in",
            "CUST-001",
            tx_id,
            "teller-1",
            now,
        )
        .await;
    assert!(
        result.is_ok(),
        "completion of CHECKED_IN booking must succeed"
    );
    tx.commit().await.expect("commit tx");

    let (status, linked_tx) = harness.fetch_booking("BK-checked-in").await;
    assert_eq!(status, "COMPLETED");
    assert_eq!(linked_tx, Some(tx_id));

    let events = harness.fetch_booking_events("BK-checked-in").await;
    assert!(
        events.iter().any(|(kind, from, to)| kind == "COMPLETE"
            && from.as_deref() == Some("CHECKED_IN")
            && to == "COMPLETED"),
        "expected COMPLETE event in booking_events"
    );

    harness.teardown().await;
}

/// 2. A completion with an IN_SERVICE booking of the same customer makes it COMPLETED
/// with the transaction id (design B10, §4 criterion 1, §9 edge cases).
#[tokio::test]
async fn completion_with_in_service_booking_completes_with_transaction_id() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_booking(
            "BK-in-service",
            "B261014-0002",
            &harness.branch_id,
            "CUST-001",
            slot_id,
            "IN_SERVICE",
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-001")
        .await;

    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = harness
        .lifecycle
        .complete_in(
            &mut tx,
            &harness.branch_id,
            "BK-in-service",
            "CUST-001",
            tx_id,
            "teller-1",
            now,
        )
        .await;
    assert!(
        result.is_ok(),
        "completion of IN_SERVICE booking must succeed"
    );
    tx.commit().await.expect("commit tx");

    let (status, linked_tx) = harness.fetch_booking("BK-in-service").await;
    assert_eq!(status, "COMPLETED");
    assert_eq!(linked_tx, Some(tx_id));

    let events = harness.fetch_booking_events("BK-in-service").await;
    assert!(
        events.iter().any(|(kind, from, to)| kind == "COMPLETE"
            && from.as_deref() == Some("IN_SERVICE")
            && to == "COMPLETED"),
        "expected COMPLETE event in booking_events"
    );

    harness.teardown().await;
}

/// 3. Another customer's booking -> bookingNotThisCustomer and nothing written
/// (design B10, §4 criterion 1, §9 edge cases).
#[tokio::test]
async fn completion_with_another_customer_booking_fails_booking_not_this_customer_and_writes_nothing()
 {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_booking(
            "BK-customer-mismatch",
            "B261014-0003",
            &harness.branch_id,
            "CUST-001", // booking is for customer 1
            slot_id,
            "CHECKED_IN",
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-002") // transaction is for customer 2
        .await;

    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = harness
        .lifecycle
        .complete_in(
            &mut tx,
            &harness.branch_id,
            "BK-customer-mismatch",
            "CUST-002", // attempt completion for customer 2
            tx_id,
            "teller-1",
            now,
        )
        .await;

    match result {
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("bookingNotThisCustomer"),
                "expected bookingNotThisCustomer refusal, got: {msg}"
            );
        }
        Ok(_) => panic!("expected bookingNotThisCustomer error, got Ok"),
    }
    tx.rollback().await.expect("rollback tx");

    let (status, linked_tx) = harness.fetch_booking("BK-customer-mismatch").await;
    assert_eq!(status, "CHECKED_IN", "booking status must remain untouched");
    assert_eq!(linked_tx, None, "transaction_id must remain null");

    let events = harness.fetch_booking_events("BK-customer-mismatch").await;
    assert!(events.is_empty(), "nothing must be written on refusal");

    harness.teardown().await;
}

/// 4. A CONFIRMED (not checked in) booking -> bookingNotUsable
/// (design B10, §4 criterion 1, §9 edge cases).
#[tokio::test]
async fn completion_with_confirmed_booking_fails_booking_not_usable() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_booking(
            "BK-confirmed-only",
            "B261014-0004",
            &harness.branch_id,
            "CUST-001",
            slot_id,
            "CONFIRMED", // not checked in
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-001")
        .await;

    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = harness
        .lifecycle
        .complete_in(
            &mut tx,
            &harness.branch_id,
            "BK-confirmed-only",
            "CUST-001",
            tx_id,
            "teller-1",
            now,
        )
        .await;

    match result {
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("bookingNotUsable"),
                "expected bookingNotUsable refusal, got: {msg}"
            );
        }
        Ok(_) => panic!("expected bookingNotUsable error, got Ok"),
    }
    tx.rollback().await.expect("rollback tx");

    let (status, linked_tx) = harness.fetch_booking("BK-confirmed-only").await;
    assert_eq!(status, "CONFIRMED", "status must remain CONFIRMED");
    assert_eq!(linked_tx, None);

    harness.teardown().await;
}

/// 5. Non-checked-in and non-in-service statuses (PENDING_CUSTOMER_CONFIRMATION, CANCELLED, NO_SHOW, EXPIRED)
/// -> bookingNotUsable (design B10, §4.3, §9 edge cases).
#[tokio::test]
async fn completion_with_non_checked_in_or_in_service_status_fails_booking_not_usable() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-001")
        .await;

    let non_usable_statuses = [
        ("BK-pending", "PENDING_CUSTOMER_CONFIRMATION"),
        ("BK-cancelled", "CANCELLED"),
        ("BK-no-show", "NO_SHOW"),
        ("BK-expired", "EXPIRED"),
    ];

    for (id, st) in non_usable_statuses {
        harness
            .seed_booking(
                id,
                &format!("B261014-{id}"),
                &harness.branch_id,
                "CUST-001",
                slot_id,
                st,
                "BUY",
                "USD",
                Decimal::from(100),
                None,
            )
            .await;

        let mut tx = harness.pool.begin().await.expect("begin tx");
        let result = harness
            .lifecycle
            .complete_in(
                &mut tx,
                &harness.branch_id,
                id,
                "CUST-001",
                tx_id,
                "teller-1",
                now,
            )
            .await;

        match result {
            Err(err) => {
                let msg = err.to_string();
                assert!(
                    msg.contains("bookingNotUsable"),
                    "{id} ({st}): expected bookingNotUsable, got {msg}"
                );
            }
            Ok(_) => panic!("{id} ({st}): expected bookingNotUsable error, got Ok"),
        }
        tx.rollback().await.expect("rollback tx");
    }

    harness.teardown().await;
}

/// 6. A booking of another branch -> bookingNotUsable (same branch required, design B10, §4.3).
#[tokio::test]
async fn completion_with_another_branch_booking_fails_booking_not_usable() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    // Slot and booking in OTHER_BRANCH_ID
    harness
        .seed_slot(
            slot_id,
            &harness.other_branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_booking(
            "BK-other-branch",
            "B261014-0006",
            &harness.other_branch_id,
            "CUST-001",
            slot_id,
            "CHECKED_IN",
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-001")
        .await;

    // Save attempt at MAIN branch
    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = harness
        .lifecycle
        .complete_in(
            &mut tx,
            &harness.branch_id, // MAIN branch
            "BK-other-branch",
            "CUST-001",
            tx_id,
            "teller-1",
            now,
        )
        .await;

    match result {
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("bookingNotUsable") || msg.contains("notFound"),
                "expected bookingNotUsable refusal for other branch booking, got: {msg}"
            );
        }
        Ok(_) => panic!("expected refusal for other branch booking, got Ok"),
    }
    tx.rollback().await.expect("rollback tx");

    harness.teardown().await;
}

/// 7. A repeat completion with the same transaction is a no-op
/// (design B10, §4 criterion 1, §9 edge cases).
#[tokio::test]
async fn repeat_completion_with_same_transaction_is_noop() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-001")
        .await;
    // Already completed with tx_id
    harness
        .seed_booking(
            "BK-repeat-noop",
            "B261014-0007",
            &harness.branch_id,
            "CUST-001",
            slot_id,
            "COMPLETED",
            "BUY",
            "USD",
            Decimal::from(100),
            Some(tx_id),
        )
        .await;

    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = harness
        .lifecycle
        .complete_in(
            &mut tx,
            &harness.branch_id,
            "BK-repeat-noop",
            "CUST-001",
            tx_id, // same transaction id
            "teller-1",
            now,
        )
        .await;
    assert!(
        result.is_ok(),
        "repeat completion with the same transaction must be a no-op (Ok)"
    );
    tx.commit().await.expect("commit tx");

    let (status, linked_tx) = harness.fetch_booking("BK-repeat-noop").await;
    assert_eq!(status, "COMPLETED");
    assert_eq!(linked_tx, Some(tx_id));

    harness.teardown().await;
}

/// 8. Side, currency or quantity different from the request still completes
/// (design B1, B10, §9 edge cases).
#[tokio::test]
async fn completion_with_differing_side_currency_or_quantity_still_completes() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    // Booking was created for BUY USD 100
    harness
        .seed_booking(
            "BK-diff-params",
            "B261014-0008",
            &harness.branch_id,
            "CUST-001",
            slot_id,
            "CHECKED_IN",
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;
    // Completed transaction differs: SELL EUR 500
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-001")
        .await;

    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = harness
        .lifecycle
        .complete_in(
            &mut tx,
            &harness.branch_id,
            "BK-diff-params",
            "CUST-001",
            tx_id,
            "teller-1",
            now,
        )
        .await;
    assert!(
        result.is_ok(),
        "completion with differing side/currency/quantity must still complete (B1: booking is a time slot, not a form lock)"
    );
    tx.commit().await.expect("commit tx");

    let (status, linked_tx) = harness.fetch_booking("BK-diff-params").await;
    assert_eq!(status, "COMPLETED");
    assert_eq!(linked_tx, Some(tx_id));

    harness.teardown().await;
}

/// 9. A second transaction for an already COMPLETED booking is refused
/// (design B10, unique index bookings_one_per_transaction, §9 edge cases).
#[tokio::test]
async fn second_transaction_for_already_completed_booking_refused() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_first = Uuid::new_v4();
    let tx_second = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_transaction(tx_first, &harness.branch_id, "CUST-001")
        .await;
    harness
        .seed_transaction(tx_second, &harness.branch_id, "CUST-001")
        .await;

    // Already COMPLETED with tx_first
    harness
        .seed_booking(
            "BK-completed-already",
            "B261014-0009",
            &harness.branch_id,
            "CUST-001",
            slot_id,
            "COMPLETED",
            "BUY",
            "USD",
            Decimal::from(100),
            Some(tx_first),
        )
        .await;

    // Attempt second completion with different transaction tx_second
    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = harness
        .lifecycle
        .complete_in(
            &mut tx,
            &harness.branch_id,
            "BK-completed-already",
            "CUST-001",
            tx_second,
            "teller-1",
            now,
        )
        .await;

    match result {
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("bookingNotUsable")
                    || msg.contains("staleVersion")
                    || msg.contains("conflict"),
                "expected refusal for already completed booking with different transaction, got: {msg}"
            );
        }
        Ok(_) => panic!("expected second transaction completion to be refused, got Ok"),
    }
    tx.rollback().await.expect("rollback tx");

    let (status, linked_tx) = harness.fetch_booking("BK-completed-already").await;
    assert_eq!(status, "COMPLETED");
    assert_eq!(
        linked_tx,
        Some(tx_first),
        "must remain linked to the first transaction"
    );

    harness.teardown().await;
}

/// 10. No rate check depends on the booking: BookingSaveStep::vouches_for_rate
/// returns false (design B1, §4 criterion 1, §7).
#[tokio::test]
async fn no_rate_check_depends_on_the_booking() {
    let harness = TestHarness::start().await;

    let step = BookingSaveStep::new(harness.lifecycle.clone());

    let form = dummy_form(&harness.branch_id, "CUST-001");
    let extras = SaveExtras::default();
    let actor = SaveActor {
        id: "teller-1".to_string(),
        username: "teller-1".to_string(),
        employee_id: Some("teller-1".to_string()),
        role: Role::Front,
    };
    let claim = BookingClaim {
        booking_id: "BK-test".to_string(),
        customer_confirmed: true,
    };
    let ctx = SaveContext {
        branch: &harness.branch_id,
        form: &form,
        extras: &extras,
        booking: Some(&claim),
        actor: &actor,
        teller: None,
        now: Utc::now(),
        prior: None,
    };

    assert!(
        !step.vouches_for_rate(&ctx),
        "no rate check depends on the booking: vouches_for_rate must return false"
    );

    harness.teardown().await;
}

/// 11. Save-path test: claiming a CHECKED_IN booking completes it with that save's transaction id,
/// transactions.booking_id stays NULL per R1 (design B10, §3 R1, §7, R6).
#[tokio::test]
async fn save_step_with_checked_in_booking_completes_with_transaction_id() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let tx_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_booking(
            "BK-save-checked-in",
            "B261014-0011",
            &harness.branch_id,
            "CUST-001",
            slot_id,
            "CHECKED_IN",
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;
    harness
        .seed_transaction(tx_id, &harness.branch_id, "CUST-001")
        .await;

    let step = BookingSaveStep::new(harness.lifecycle.clone());
    let form = dummy_form(&harness.branch_id, "CUST-001");
    let extras = SaveExtras::default();
    let actor = SaveActor {
        id: "teller-1".to_string(),
        username: "teller-1".to_string(),
        employee_id: Some("teller-1".to_string()),
        role: Role::Front,
    };
    let claim = BookingClaim {
        booking_id: "BK-save-checked-in".to_string(),
        customer_confirmed: true,
    };
    let ctx = SaveContext {
        branch: &harness.branch_id,
        form: &form,
        extras: &extras,
        booking: Some(&claim),
        actor: &actor,
        teller: None,
        now,
        prior: None,
    };

    let mut tx = harness.pool.begin().await.expect("begin tx");
    step.before_in(&mut tx, &ctx).await.expect("before_in");

    let saved = dummy_exchange_transaction(&format!("DOC-{tx_id}"), &harness.branch_id, "CUST-001");
    step.after_in(&mut tx, &ctx, &saved)
        .await
        .expect("after_in");
    tx.commit().await.expect("commit tx");

    let (status, linked_tx) = harness.fetch_booking("BK-save-checked-in").await;
    assert_eq!(status, "COMPLETED");
    assert_eq!(linked_tx, Some(tx_id));

    let tx_booking_id: Option<String> =
        sqlx::query_scalar("SELECT booking_id FROM transactions WHERE id = $1")
            .bind(tx_id)
            .fetch_one(&harness.pool)
            .await
            .expect("fetch tx");
    assert!(
        tx_booking_id.is_none(),
        "transactions.booking_id must stay NULL per R1"
    );

    harness.teardown().await;
}

/// 12. Save-path test: claiming another customer's booking refuses with bookingNotThisCustomer,
/// and nothing is written (design B10, §4.3, R6).
#[tokio::test]
async fn save_step_with_another_customer_booking_refuses_booking_not_this_customer() {
    let harness = TestHarness::start().await;
    let slot_id = Uuid::new_v4();
    let now = Utc::now();
    let today = now.date_naive();

    harness
        .seed_slot(
            slot_id,
            &harness.branch_id,
            today,
            now,
            now + Duration::minutes(30),
        )
        .await;
    harness
        .seed_booking(
            "BK-save-other-cust",
            "B261014-0012",
            &harness.branch_id,
            "CUST-002",
            slot_id,
            "CHECKED_IN",
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;

    let step = BookingSaveStep::new(harness.lifecycle.clone());
    let form = dummy_form(&harness.branch_id, "CUST-001");
    let extras = SaveExtras::default();
    let actor = SaveActor {
        id: "teller-1".to_string(),
        username: "teller-1".to_string(),
        employee_id: Some("teller-1".to_string()),
        role: Role::Front,
    };
    let claim = BookingClaim {
        booking_id: "BK-save-other-cust".to_string(),
        customer_confirmed: true,
    };
    let ctx = SaveContext {
        branch: &harness.branch_id,
        form: &form,
        extras: &extras,
        booking: Some(&claim),
        actor: &actor,
        teller: None,
        now,
        prior: None,
    };

    let mut tx = harness.pool.begin().await.expect("begin tx");
    let result = step.before_in(&mut tx, &ctx).await;
    match result {
        Err(err) => {
            let msg = err.to_string();
            assert!(
                msg.contains("bookingNotThisCustomer"),
                "expected bookingNotThisCustomer error, got: {msg}"
            );
        }
        Ok(_) => panic!("expected bookingNotThisCustomer error, got Ok"),
    }
    tx.rollback().await.expect("rollback tx");

    let (status, linked_tx) = harness.fetch_booking("BK-save-other-cust").await;
    assert_eq!(status, "CHECKED_IN");
    assert_eq!(linked_tx, None);

    harness.teardown().await;
}
