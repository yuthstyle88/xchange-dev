//! G7 — up-front tests: completing a booked customer's exchange moves the booking's referral capture to the exchange
//! (design `docs/design/blueprint/booking-slots.md` B13: *"The capture moves to the exchange with
//! `link_transaction_in`"*; `docs/design/v9-v3-comparison-mapping.md` §3; G7 on #5055).
//!
//! Written before the implementation by the ticket author (claude/bc8fd29a, xchange BE lane controller).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The defect at origin/main `2bd6d41ab`: `ReferralCaptureService::link_transaction_in`
//! (`xc-referral/src/application/capture_service.rs:278`) has no caller in `api/src` or `api/crates`. So a referral code
//! captured on a DESK or APP booking keeps `transaction_id = NULL` after the booking's exchange completes, and the
//! exchange carries no attribution.
//!
//! The seam: `BookingSaveStep::with_referral_capture(self, Arc<ReferralCaptureService>) -> Self` (a builder, so every
//! existing `BookingSaveStep::new(lifecycle)` caller and test keeps compiling). With it set, `after_in` calls
//! `link_transaction_in(tx, booking_id, transaction_id)` after `complete_in`, on the save's transaction.
//!
//! What this file pins ([required]):
//! - L1: a booking with an ACTIVE capture: after the save, that capture's `transaction_id` is the save's transaction,
//!   it stays the one ACTIVE row, and the booking is COMPLETED.
//! - L2: a booking with no capture: the save completes as before and no capture row appears.
//! - L3: another booking's capture is untouched.
//! - L4: a refused save (another customer's booking) leaves the capture unlinked.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use uuid::Uuid;

use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_booking::application::lifecycle::BookingLifecycle;
use xc_booking::infrastructure::booking_repository::PgBookingRepository;
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_customer::domain::customer::value_objects::customer_details::CustomerDetails;
use xc_exchange::domain::exchange::entities::transaction::Transaction as ExchangeTransaction;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{ForeignAmount, Money, Quote, Rate, Side};
use xchange::application::workflows::booking_save_step::BookingSaveStep;
use xchange::application::workflows::save_step::{
    BookingClaim, SaveActor, SaveContext, SaveExtras, SaveStep,
};

const MUST_FINISH: StdDuration = StdDuration::from_secs(60);
const ORG_ID: &str = "00000000-0000-0000-0000-000000000001";
const BRANCH_ID: &str = "MAIN";
const OTHER_BRANCH_ID: &str = "B02";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("g7link");
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
            "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, status, submission_id, payload_hash, \
                customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
                recorded_at, recorded_by, completed_at, count_confirmed) VALUES ($1, $2, $3, $4, 'DEMO', 'COMPLETED', $5, 'payload-hash', $6, 'Test Customer', 'THA', \
                'teller-1', 'B261014-0001', '', '2026-10-14', now(), now(), 'teller-1', now(), true) ON CONFLICT (id) DO NOTHING",
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
        amount: ForeignAmount::from_decimal(Decimal::from_str("100.00").unwrap()),
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

use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_referral::application::capture_service::ReferralCaptureService;
use xc_referral::infrastructure::capture_repository::PgCaptureRepository;

impl TestHarness {
    /// One ACTIVE `DESK` capture on `booking_id` for `customer_id`, with no transaction yet.
    async fn seed_capture(&self, booking_id: &str, customer_id: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO referral_attributions (id, organization_id, branch_id, code, customer_id, booking_id, \
                source, captured_at, captured_by, status) \
             VALUES ($1, $2, $3, 'REFER123', $4, $5, 'DESK', now(), 'teller-1', 'ACTIVE')",
        )
        .bind(id)
        .bind(self.org_id)
        .bind(self.branch_id.as_str())
        .bind(customer_id)
        .bind(booking_id)
        .execute(&self.pool)
        .await
        .expect("seed an ACTIVE capture on the booking");
        id
    }

    /// `(status, transaction_id)` of every capture on `booking_id`.
    async fn captures_of(&self, booking_id: &str) -> Vec<(String, Option<Uuid>)> {
        sqlx::query(
            "SELECT status, transaction_id FROM referral_attributions WHERE booking_id = $1",
        )
        .bind(booking_id)
        .fetch_all(&self.pool)
        .await
        .expect("read captures")
        .into_iter()
        .map(|r| (r.get("status"), r.get("transaction_id")))
        .collect()
    }

    fn step(&self) -> BookingSaveStep {
        BookingSaveStep::new(self.lifecycle.clone()).with_referral_capture(Arc::new(
            ReferralCaptureService::new(
                Arc::new(PgCaptureRepository::new()),
                Arc::new(PgAuditLog::new(self.pool.clone())),
            ),
        ))
    }

    /// A CHECKED_IN booking of `customer` on a fresh slot, and a seeded transaction `tx_id`.
    async fn booked(&self, booking_id: &str, booking_no: &str, customer: &str, tx_id: Uuid) {
        let slot_id = Uuid::new_v4();
        let now = Utc::now();
        self.seed_slot(
            slot_id,
            &self.branch_id,
            now.date_naive(),
            now,
            now + Duration::minutes(30),
        )
        .await;
        self.seed_booking(
            booking_id,
            booking_no,
            &self.branch_id,
            customer,
            slot_id,
            "CHECKED_IN",
            "BUY",
            "USD",
            Decimal::from(100),
            None,
        )
        .await;
        self.seed_transaction(tx_id, &self.branch_id, customer)
            .await;
    }
}

fn actor() -> SaveActor {
    SaveActor {
        id: "teller-1".to_string(),
        username: "teller-1".to_string(),
        employee_id: Some("teller-1".to_string()),
        standing: StandingKind::Staff(AccessRole::Teller),
    }
}

/// Runs the step's `before_in` + `after_in` for `customer` claiming `booking_id`, committing only when both pass.
async fn save(
    harness: &TestHarness,
    booking_id: &str,
    customer: &str,
    tx_id: Uuid,
) -> Result<(), String> {
    let step = harness.step();
    let form = dummy_form(&harness.branch_id, customer);
    let extras = SaveExtras::default();
    let actor = actor();
    let claim = BookingClaim {
        booking_id: booking_id.to_string(),
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
    let mut tx = harness.pool.begin().await.expect("begin tx");
    let outcome = async {
        step.before_in(&mut tx, &ctx).await?;
        let saved =
            dummy_exchange_transaction(&format!("DOC-{tx_id}"), &harness.branch_id, customer);
        step.after_in(&mut tx, &ctx, &saved).await
    }
    .await;
    match outcome {
        Ok(()) => {
            tx.commit().await.expect("commit tx");
            Ok(())
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(format!("{e:?}"))
        }
    }
}

/// L1: the booking's ACTIVE capture gets the save's transaction id, and stays the one ACTIVE row.
#[tokio::test]
async fn l1_completing_a_booked_exchange_links_the_bookings_capture_to_the_transaction() {
    let harness = TestHarness::start().await;
    let tx_id = Uuid::new_v4();
    harness
        .booked("BK-g7-l1", "B261025-0001", "CUST-001", tx_id)
        .await;
    harness.seed_capture("BK-g7-l1", "CUST-001").await;

    save(&harness, "BK-g7-l1", "CUST-001", tx_id)
        .await
        .expect("the booked save passes");

    let (status, linked) = harness.fetch_booking("BK-g7-l1").await;
    assert_eq!(status, "COMPLETED");
    assert_eq!(linked, Some(tx_id));
    assert_eq!(
        harness.captures_of("BK-g7-l1").await,
        vec![("ACTIVE".to_string(), Some(tx_id))],
        "the booking's one ACTIVE capture now names the save's transaction"
    );

    harness.teardown().await;
}

/// L2: a booking with no capture completes as before, and no capture row appears.
#[tokio::test]
async fn l2_a_booking_with_no_capture_completes_and_writes_no_capture() {
    let harness = TestHarness::start().await;
    let tx_id = Uuid::new_v4();
    harness
        .booked("BK-g7-l2", "B261025-0002", "CUST-001", tx_id)
        .await;

    save(&harness, "BK-g7-l2", "CUST-001", tx_id)
        .await
        .expect("the booked save passes");

    assert_eq!(harness.fetch_booking("BK-g7-l2").await.0, "COMPLETED");
    assert!(harness.captures_of("BK-g7-l2").await.is_empty());

    harness.teardown().await;
}

/// L3: completing one booking leaves another booking's capture unlinked.
#[tokio::test]
async fn l3_another_bookings_capture_is_untouched() {
    let harness = TestHarness::start().await;
    let tx_id = Uuid::new_v4();
    let other_tx = Uuid::new_v4();
    harness
        .booked("BK-g7-l3", "B261025-0003", "CUST-001", tx_id)
        .await;
    harness
        .booked("BK-g7-l3-other", "B261025-0004", "CUST-002", other_tx)
        .await;
    harness.seed_capture("BK-g7-l3", "CUST-001").await;
    harness.seed_capture("BK-g7-l3-other", "CUST-002").await;

    save(&harness, "BK-g7-l3", "CUST-001", tx_id)
        .await
        .expect("the booked save passes");

    assert_eq!(
        harness.captures_of("BK-g7-l3-other").await,
        vec![("ACTIVE".to_string(), None)],
        "the other booking's capture keeps no transaction"
    );

    harness.teardown().await;
}

/// L4: a refused save (another customer's booking) leaves the capture unlinked.
#[tokio::test]
async fn l4_a_refused_save_leaves_the_capture_unlinked() {
    let harness = TestHarness::start().await;
    let tx_id = Uuid::new_v4();
    harness
        .booked("BK-g7-l4", "B261025-0005", "CUST-001", tx_id)
        .await;
    harness.seed_capture("BK-g7-l4", "CUST-001").await;

    let refused = save(&harness, "BK-g7-l4", "CUST-002", tx_id).await;
    assert!(
        refused.is_err(),
        "CUST-002 cannot use CUST-001's booking: {refused:?}"
    );

    assert_eq!(
        harness.captures_of("BK-g7-l4").await,
        vec![("ACTIVE".to_string(), None)],
        "nothing is linked when the save is refused"
    );
    assert_ne!(harness.fetch_booking("BK-g7-l4").await.0, "COMPLETED");

    harness.teardown().await;
}
