//! Regression tests for #1938: `CheckInService` must determine "today" in the branch's timezone
//! (`branches.timezone`), not the organisation's (`organizations.timezone`).

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Bangkok;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_booking::application::check_in::{CheckInBy, CheckInError, CheckInService};
use xc_booking::application::lifecycle::BookingLifecycle;
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::infrastructure::booking_repository::PgBookingRepository;
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::PiiKeys;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::domain::repository::QueueRepository;
use xc_queue::infrastructure::repository::PgQueueRepository;

const MUST_FINISH: StdDuration = StdDuration::from_secs(60);
const ORG_ID: &str = "00000000-0000-0000-0000-000000000001";
const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";
const BRANCH_ID: &str = "XC-B01";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

fn pii_keys() -> PiiKeys {
    PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("valid keys")
}

struct TestHarness {
    pool: PgPool,
    schema: String,
    branch_id: BranchId,
    org_id: Uuid,
    check_in_service: CheckInService,
}

impl TestHarness {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The booking check-in tests need a real PostgreSQL and do not skip."
            )
        });
        let options =
            PgConnectOptions::from_str(&url).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("bk_tz_test");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");

        let dir = migrations_dir();
        let migrator = Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
        migrator
            .run(&pool)
            .await
            .unwrap_or_else(|e| panic!("every migration applies on a fresh database: {e}"));

        let org_uuid = Uuid::parse_str(ORG_ID).unwrap();
        let branch = BranchId::new(BRANCH_ID);

        sqlx::query(
            "INSERT INTO currencies (code, name_th, name_en, symbol) \
             VALUES ('USD', 'US Dollar', 'US Dollar', '$'), ('THB', 'Thai Baht', 'Thai Baht', '฿') \
             ON CONFLICT (code) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed currencies");

        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, booking_enabled) \
             VALUES ($1, 'Test Branch', $2, true) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(branch.as_str())
        .bind(org_uuid)
        .execute(&pool)
        .await
        .expect("seed branch");

        sqlx::query(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open) \
             VALUES ($1, $2, true) \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true",
        )
        .bind(branch.as_str())
        .bind(org_uuid)
        .execute(&pool)
        .await
        .expect("seed branch_service_status");

        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'Teller 1', 'ACTIVE', 'test') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(TELLER_ID)
        .bind(org_uuid)
        .execute(&pool)
        .await
        .expect("seed teller user");

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('shift-1', $1, 'OPEN', now(), 'test', 'req-open', $2::uuid) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(branch.as_str())
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("seed shift");

        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             VALUES ($1, $2, $3::uuid, 'ACTIVE') \
             ON CONFLICT (branch_id, teller_id) DO UPDATE SET status = 'ACTIVE'",
        )
        .bind(org_uuid)
        .bind(branch.as_str())
        .bind(TELLER_ID)
        .execute(&pool)
        .await
        .expect("seed queue counter");

        let transactor = pos_infra::transactions::Transactor::new(pool.clone());
        let branch_tx = BranchTx::new(transactor);
        let queue_repo: Arc<dyn QueueRepository> = Arc::new(PgQueueRepository::new(pool.clone()));
        let booking_repo = Arc::new(PgBookingRepository::new(pool.clone()));
        let slot_repo = Arc::new(PgSlotRepository::new(pool.clone()));
        let booking_lifecycle = Arc::new(BookingLifecycle::new(booking_repo.clone()));

        let teller_actor = Actor::new(
            TELLER_ID,
            ORG_ID,
            vec![ScopedRole::new(
                Role::Teller,
                Some(branch.clone()),
                Utc::now() - Duration::days(1),
                None,
            )],
            None,
        );

        let issuer = TicketIssuer::new(branch_tx.clone(), queue_repo)
            .with_booking_events(booking_lifecycle)
            .acting_as(teller_actor);

        let check_in_service = CheckInService::new(
            pool.clone(),
            branch_tx,
            booking_repo,
            slot_repo,
            Arc::new(issuer),
            pii_keys(),
        );

        Self {
            pool,
            schema,
            branch_id: branch,
            org_id: org_uuid,
            check_in_service,
        }
    }

    async fn finish(self) {
        let Self { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }

    async fn set_timezones(&self, branch_tz: &str, org_tz: &str) {
        sqlx::query("UPDATE branches SET timezone = $1 WHERE id = $2")
            .bind(branch_tz)
            .bind(self.branch_id.as_str())
            .execute(&self.pool)
            .await
            .expect("set branch timezone");
        sqlx::query("UPDATE organizations SET timezone = $1 WHERE id = $2")
            .bind(org_tz)
            .bind(self.org_id)
            .execute(&self.pool)
            .await
            .expect("set organization timezone");
    }

    async fn seed_customer(&self, customer_id: &str, display_name: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2, $1, $3, 'LIVE', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(customer_id)
        .bind(self.org_id)
        .bind(display_name)
        .execute(&self.pool)
        .await
        .expect("seed customer");
    }

    async fn seed_slot(
        &self,
        slot_id: Uuid,
        service_date: NaiveDate,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
    ) {
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             VALUES ($1, $2, $3, $4, $5, $6, 5, 1, 'OPEN') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(slot_id)
        .bind(self.org_id)
        .bind(self.branch_id.as_str())
        .bind(service_date)
        .bind(start_at)
        .bind(end_at)
        .execute(&self.pool)
        .await
        .expect("seed slot");
    }

    async fn seed_booking(
        &self,
        booking_id: &str,
        booking_no: &str,
        customer_id: &str,
        slot_id: Uuid,
        qr_token_hash: &[u8],
    ) {
        sqlx::query(
            "INSERT INTO bookings (\
                id, organization_id, branch_id, booking_no, customer_id, slot_id, \
                shop_side, requested_currency, requested_quantity, source_channel, \
                status, qr_token_hash, qr_version, created_by\
             ) VALUES ($1, $2, $3, $4, $5, $6, 'BUY', 'USD', 100.0000, 'APP', 'CONFIRMED', $7, 1, 'seed') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(booking_id)
        .bind(self.org_id)
        .bind(self.branch_id.as_str())
        .bind(booking_no)
        .bind(customer_id)
        .bind(slot_id)
        .bind(qr_token_hash)
        .execute(&self.pool)
        .await
        .expect("seed booking");
    }
}

fn make_qr_token() -> (String, Vec<u8>) {
    let token = format!("qr-token-{}", Uuid::new_v4());
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let hash = hasher.finalize().to_vec();
    (token, hash)
}

#[tokio::test]
async fn branch_bangkok_org_utc_past_midnight_in_bangkok_admits_slot_on_branch_today() {
    let h = TestHarness::start().await;
    h.set_timezones("Asia/Bangkok", "UTC").await;

    let now = DateTime::parse_from_rfc3339("2030-06-01T18:30:00Z")
        .expect("valid RFC3339")
        .with_timezone(&Utc);
    let service_date = NaiveDate::from_ymd_opt(2030, 6, 2).expect("valid date");
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Alice").await;
    h.seed_slot(
        slot_id,
        service_date,
        now - Duration::minutes(5),
        now + Duration::minutes(25),
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B300602-0001",
        &customer_id,
        slot_id,
        &token_hash,
    )
    .await;

    let (booking, ticket) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token),
            TELLER_ID,
            now,
        )
        .await
        .expect("check-in must succeed when slot service_date is today at the branch");

    assert_eq!(booking.status, BookingStatus::CheckedIn);
    assert_eq!(ticket.service_type, "BOOKING");

    h.finish().await;
}

#[tokio::test]
async fn branch_bangkok_org_utc_past_midnight_in_bangkok_refuses_slot_on_org_date() {
    let h = TestHarness::start().await;
    h.set_timezones("Asia/Bangkok", "UTC").await;

    let now = DateTime::parse_from_rfc3339("2030-06-01T18:30:00Z")
        .expect("valid RFC3339")
        .with_timezone(&Utc);
    let service_date = NaiveDate::from_ymd_opt(2030, 6, 1).expect("valid date");
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Bob").await;
    h.seed_slot(
        slot_id,
        service_date,
        now - Duration::minutes(5),
        now + Duration::minutes(25),
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B300601-0001",
        &customer_id,
        slot_id,
        &token_hash,
    )
    .await;

    let err = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token),
            TELLER_ID,
            now,
        )
        .await
        .expect_err("slot dated 2030-06-01 must be refused when branch local date is 2030-06-02");

    match err {
        CheckInError::CheckInWindow => {}
        other => panic!("expected CheckInWindow, got {other:?}"),
    }

    h.finish().await;
}

#[tokio::test]
async fn branch_and_org_both_bangkok_admits_slot_today_within_grace() {
    let h = TestHarness::start().await;
    h.set_timezones("Asia/Bangkok", "Asia/Bangkok").await;

    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Charlie").await;
    h.seed_slot(
        slot_id,
        service_date,
        now - Duration::minutes(5),
        now + Duration::minutes(25),
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B261015-0001",
        &customer_id,
        slot_id,
        &token_hash,
    )
    .await;

    let (booking, ticket) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token),
            TELLER_ID,
            now,
        )
        .await
        .expect("check-in must succeed when branch and org are both Bangkok and slot is today");

    assert_eq!(booking.status, BookingStatus::CheckedIn);
    assert_eq!(ticket.service_type, "BOOKING");

    h.finish().await;
}
