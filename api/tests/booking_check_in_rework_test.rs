//! Tests for BK-4 rework behaviours:
//! 1. Re-check-in is allowed only when the previous ticket ended NO_SHOW (design B8).
//! 2. Teller cancel of a CHECKED_IN booking writes ended_reason = 'bookingCancelled' on the ticket and event.
//! 3. Day close releases slot place through SlotRepository::release_in.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration as StdDuration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Asia::Bangkok;
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use uuid::Uuid;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_booking::application::booking_service::BookingService;
use xc_booking::application::check_in::{CheckInBy, CheckInError, CheckInService};
use xc_booking::application::lifecycle::BookingLifecycle;
use xc_booking::infrastructure::booking_repository::PgBookingRepository;
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_customer::application::customer::ContactReserve;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::PiiKeys;
use xc_queue::application::day::queue_day_hook::QueueDayHook;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::application::status::status_service::StatusService;
use xc_queue::domain::repository::QueueRepository;
use xc_queue::infrastructure::repository::PgQueueRepository;

const MUST_FINISH: StdDuration = StdDuration::from_secs(60);
const ORG_ID: &str = "00000000-0000-0000-0000-000000000001";
const TELLER_ID: &str = "00000000-0000-0000-0000-000000000010";
const BRANCH_ID: &str = "XC-B01";

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

fn pii_keys() -> PiiKeys {
    PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("valid keys")
}

struct TestHarness {
    pool: PgPool,
    schema: String,
    branch_id: BranchId,
    org_id: Uuid,
    day_hook: QueueDayHook,
    check_in_service: CheckInService,
    booking_service: BookingService,
}

impl TestHarness {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The booking check-in tests need a real PostgreSQL and do not skip.\n\
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
        let schema = unique("bk4rw");
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
            .expect("read migrations directory");
        migrator.run(&pool).await.expect("run migrations");

        let org_uuid = Uuid::parse_str(ORG_ID).unwrap();
        let branch = BranchId::new(BRANCH_ID);

        // Seed currencies
        sqlx::query(
            "INSERT INTO currencies (code, name_th, name_en, symbol) \
             VALUES ('USD', 'US Dollar', 'US Dollar', '$'), ('THB', 'Thai Baht', 'Thai Baht', '฿') \
             ON CONFLICT (code) DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("seed currencies");

        // Seed branch
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

        // Seed branch_service_status (queue open by default)
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

        // Seed teller user
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

        // Shift and counter for teller
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
        let status_service = StatusService::new(branch_tx.clone(), queue_repo.clone());

        let booking_lifecycle =
            Arc::new(BookingLifecycle::new(booking_repo.clone()).with_slots(slot_repo.clone()));

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

        let issuer = TicketIssuer::new(branch_tx.clone(), queue_repo.clone())
            .with_booking_events(booking_lifecycle.clone())
            .acting_as(teller_actor);

        let day_hook = QueueDayHook::new(queue_repo.clone(), status_service.clone())
            .with_booking_events(booking_lifecycle.clone());

        let check_in_service = CheckInService::new(
            pool.clone(),
            branch_tx.clone(),
            booking_repo.clone(),
            slot_repo.clone(),
            Arc::new(issuer.clone()),
            pii_keys(),
        );

        let contact_reserve = Arc::new(ContactReserve::new(branch_tx.clone()));
        let booking_service = BookingService::new(
            pool.clone(),
            booking_repo.clone(),
            slot_repo.clone(),
            contact_reserve,
            None,
            pii_keys(),
        )
        .with_queue(Arc::new(issuer.clone()));

        Self {
            pool,
            schema,
            branch_id: branch,
            org_id: org_uuid,
            day_hook,
            check_in_service,
            booking_service,
        }
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
        capacity: i32,
        reserved: i32,
    ) {
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'OPEN')",
        )
        .bind(slot_id)
        .bind(self.org_id)
        .bind(self.branch_id.as_str())
        .bind(service_date)
        .bind(start_at)
        .bind(end_at)
        .bind(capacity)
        .bind(reserved)
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
        status: &str,
        qr_token_hash: &[u8],
    ) {
        sqlx::query(
            "INSERT INTO bookings (\
                id, organization_id, branch_id, booking_no, customer_id, slot_id, \
                shop_side, requested_currency, requested_quantity, source_channel, \
                status, qr_token_hash, qr_version, created_by\
             ) VALUES ($1, $2, $3, $4, $5, $6, 'BUY', 'USD', 100.0000, 'APP', $7, $8, 1, 'seed') \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(booking_id)
        .bind(self.org_id)
        .bind(self.branch_id.as_str())
        .bind(booking_no)
        .bind(customer_id)
        .bind(slot_id)
        .bind(status)
        .bind(qr_token_hash)
        .execute(&self.pool)
        .await
        .expect("seed booking");
    }

    async fn finish(self) {
        let Self { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

fn make_qr_token() -> (String, Vec<u8>) {
    let token = format!("qr-token-{}", Uuid::new_v4());
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    (token, hasher.finalize().to_vec())
}

#[tokio::test]
async fn re_check_in_refused_for_cancelled_or_completed_ticket() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "ReCheckInCustomer").await;
    h.seed_slot(
        slot_id,
        service_date,
        now - Duration::minutes(5),
        now + Duration::minutes(25),
        5,
        1,
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B261012-0050",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash,
    )
    .await;

    // 1. Initial check-in succeeds
    let (_, ticket) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token.clone()),
            TELLER_ID,
            now,
        )
        .await
        .expect("initial check-in succeeds");

    // 2. While ticket is CANCELLED (not NO_SHOW), re-check-in is refused (StaleVersion)
    sqlx::query("UPDATE queue_tickets SET status = 'CANCELLED' WHERE id = $1::uuid")
        .bind(&ticket.id)
        .execute(&h.pool)
        .await
        .expect("update ticket to CANCELLED");

    let err = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token.clone()),
            TELLER_ID,
            now,
        )
        .await
        .expect_err("re-check-in after CANCELLED ticket must be refused");
    assert!(
        matches!(err, CheckInError::StaleVersion),
        "expected StaleVersion, got {err:?}"
    );

    // 3. While ticket is COMPLETED, re-check-in is refused (StaleVersion)
    sqlx::query("UPDATE queue_tickets SET status = 'COMPLETED' WHERE id = $1::uuid")
        .bind(&ticket.id)
        .execute(&h.pool)
        .await
        .expect("update ticket to COMPLETED");

    let err = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token.clone()),
            TELLER_ID,
            now,
        )
        .await
        .expect_err("re-check-in after COMPLETED ticket must be refused");
    assert!(
        matches!(err, CheckInError::StaleVersion),
        "expected StaleVersion, got {err:?}"
    );

    // 4. When ticket is marked NO_SHOW, re-check-in succeeds!
    sqlx::query("UPDATE queue_tickets SET status = 'NO_SHOW' WHERE id = $1::uuid")
        .bind(&ticket.id)
        .execute(&h.pool)
        .await
        .expect("update ticket to NO_SHOW");

    let (_, ticket_2) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token),
            TELLER_ID,
            now,
        )
        .await
        .expect("re-check-in after NO_SHOW ticket must succeed");
    assert_ne!(ticket.id, ticket_2.id);

    h.finish().await;
}

#[tokio::test]
async fn teller_cancel_writes_ended_reason_booking_cancelled() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "CancelTestCustomer").await;
    h.seed_slot(
        slot_id,
        service_date,
        now - Duration::minutes(5),
        now + Duration::minutes(25),
        5,
        1,
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B261012-0051",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash,
    )
    .await;

    let (_, ticket) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token),
            TELLER_ID,
            now,
        )
        .await
        .expect("check-in succeeds");

    // Staff cancels the checked-in booking
    h.booking_service
        .cancel(&booking_id, TELLER_ID, true, Some("customer left"), now)
        .await
        .expect("staff cancel succeeds");

    // Verify ticket has status CANCELLED and ended_reason = 'bookingCancelled'
    let row = sqlx::query("SELECT status, ended_reason FROM queue_tickets WHERE id = $1::uuid")
        .bind(&ticket.id)
        .fetch_one(&h.pool)
        .await
        .expect("fetch ticket");
    let status: String = row.get("status");
    let ended_reason: Option<String> = row.get("ended_reason");
    assert_eq!(status, "CANCELLED");
    assert_eq!(ended_reason.as_deref(), Some("bookingCancelled"));

    // Verify cancel queue event has reason = 'bookingCancelled'
    let event_row = sqlx::query(
        "SELECT reason FROM queue_events WHERE ticket_id = $1::uuid AND kind = 'CANCEL'",
    )
    .bind(&ticket.id)
    .fetch_one(&h.pool)
    .await
    .expect("fetch cancel event");
    let reason: Option<String> = event_row.get("reason");
    assert_eq!(reason.as_deref(), Some("bookingCancelled"));

    h.finish().await;
}

#[tokio::test]
async fn day_close_releases_slot_place_via_slot_repository() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "DayCloseCustomer").await;
    h.seed_slot(
        slot_id,
        service_date,
        now - Duration::minutes(5),
        now + Duration::minutes(25),
        5,
        1,
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B261012-0052",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash,
    )
    .await;

    let (_, ticket) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token),
            TELLER_ID,
            now,
        )
        .await
        .expect("check-in succeeds");

    // Run day close hook
    let mut tx = h.pool.begin().await.expect("begin tx");
    let cancelled = h
        .day_hook
        .on_day_closed(&mut tx, &h.branch_id, now + Duration::hours(8))
        .await
        .expect("day close succeeds");
    tx.commit().await.expect("commit day close");

    assert!(cancelled.iter().any(|t| t.id == ticket.id));

    // Check slot reserved count is decremented to 0
    let slot_row = sqlx::query("SELECT reserved FROM booking_slots WHERE id = $1")
        .bind(slot_id)
        .fetch_one(&h.pool)
        .await
        .expect("fetch slot");
    let reserved: i32 = slot_row.get("reserved");
    assert_eq!(reserved, 0, "slot capacity must be released on day close");

    h.finish().await;
}
