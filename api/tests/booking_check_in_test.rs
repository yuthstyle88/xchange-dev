//! BK-4 #1382 — tests for booking check-in, `CheckInService`, `BookingQueueEvents` port,
//! and queue start / day-close hooks (design `docs/design/blueprint/booking-slots.md` §4.2, §4.3, §5, §9, §9.1).
//!
//! Written before implementation by test-writer (gemini/warut-4, ordered by claude/ad05bf8d, Owner's Rule A).
//! Do not edit, delete or weaken.
//!
//! Every row of design §9.1 BK-4 and ticket #1382 §9 is pinned by a test here:
//! 1. Right QR check-in issues BOOKING ticket and marks booking CHECKED_IN in one transaction.
//! 2. Identity check by teller issues BOOKING ticket and marks booking CHECKED_IN.
//! 3. Wrong QR token returns qrMismatch, booking stays CONFIRMED, no ticket.
//! 4. QR of another booking returns qrMismatch, both bookings stay CONFIRMED.
//! 5. Closed queue returns queueClosed, booking stays CONFIRMED, no ticket.
//! 6. Check-in on another day returns checkInWindow, booking stays CONFIRMED.
//! 7. Check-in after start_at + grace returns checkInWindow, booking stays CONFIRMED.
//! 8. Ticket moving to SERVING via QueueDesk::start_in moves booking to IN_SERVICE.
//! 9. Ticket ending NO_SHOW leaves booking CHECKED_IN; second check-in issues a second ticket.
//! 10. Day close via QueueDayHook::on_day_closed cancels CHECKED_IN booking with dayClosed.
//! 11. Teller cancel of CHECKED_IN booking cancels its active ticket via cancel_active_for_booking_in (port).
//! 12. Teller cancel of CHECKED_IN booking via BookingService::cancel cancels active ticket (service).
//!
//! Needs TEST_DATABASE_URL naming an existing database whose name ends in `_test`.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
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
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::infrastructure::booking_repository::PgBookingRepository;
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_customer::application::customer::ContactReserve;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::pii::PiiKeys;
use xc_queue::application::day::queue_day_hook::QueueDayHook;
use xc_queue::application::desk::queue_desk::QueueDesk;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::application::status::status_service::StatusService;
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
    teller_actor: Actor,
    issuer: TicketIssuer,
    desk: QueueDesk,
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
        let schema = unique("bk4test");
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

        // Wire services
        let transactor = pos_infra::transactions::Transactor::new(pool.clone());
        let branch_tx = BranchTx::new(transactor);
        let queue_repo: Arc<dyn QueueRepository> = Arc::new(PgQueueRepository::new(pool.clone()));
        let booking_repo = Arc::new(PgBookingRepository::new(pool.clone()));
        let slot_repo = Arc::new(PgSlotRepository::new(pool.clone()));
        let status_service = StatusService::new(branch_tx.clone(), queue_repo.clone());

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

        let issuer = TicketIssuer::new(branch_tx.clone(), queue_repo.clone())
            .with_booking_events(booking_lifecycle.clone())
            .acting_as(teller_actor.clone());

        let desk = QueueDesk::new(
            branch_tx.clone(),
            queue_repo.clone(),
            status_service.clone(),
        )
        .with_booking_events(booking_lifecycle.clone());

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
            teller_actor,
            issuer,
            desk,
            day_hook,
            check_in_service,
            booking_service,
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

    /// Seed customer directly
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

    /// Seed slot directly
    async fn seed_slot(
        &self,
        slot_id: Uuid,
        service_date: chrono::NaiveDate,
        start_at: DateTime<Utc>,
        end_at: DateTime<Utc>,
        capacity: i32,
        reserved: i32,
    ) {
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'OPEN') \
             ON CONFLICT (id) DO NOTHING",
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

    /// Seed booking directly (per ruling & note 9: never through BK-12, random 32-byte hash)
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

    async fn get_booking_status(
        &self,
        booking_id: &str,
    ) -> (String, Option<DateTime<Utc>>, Option<String>) {
        let row =
            sqlx::query("SELECT status, checked_in_at, ended_reason FROM bookings WHERE id = $1")
                .bind(booking_id)
                .fetch_one(&self.pool)
                .await
                .expect("fetch booking");

        (
            row.get("status"),
            row.get("checked_in_at"),
            row.get("ended_reason"),
        )
    }

    async fn count_booking_events(&self, booking_id: &str, kind: &str) -> i64 {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM booking_events WHERE booking_id = $1 AND kind = $2",
        )
        .bind(booking_id)
        .bind(kind)
        .fetch_one(&self.pool)
        .await
        .expect("count booking events");

        n
    }

    async fn get_ticket_for_booking(
        &self,
        booking_id: &str,
    ) -> Option<(String, String, String, Option<String>)> {
        let row_opt = sqlx::query(
            "SELECT id::text AS id, status, display_number, ended_reason FROM queue_tickets WHERE booking_id = $1 ORDER BY sequence DESC LIMIT 1",
        )
        .bind(booking_id)
        .fetch_optional(&self.pool)
        .await
        .expect("fetch ticket for booking");

        row_opt.map(|r| {
            (
                r.get("id"),
                r.get("status"),
                r.get("display_number"),
                r.get("ended_reason"),
            )
        })
    }
}

fn make_qr_token() -> (String, Vec<u8>) {
    let token = format!("qr-token-{}", Uuid::new_v4());
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let hash = hasher.finalize().to_vec();
    (token, hash)
}

// ------------------------------------------------------------------------------------------------
// 1. Right QR check-in issues BOOKING ticket and marks booking CHECKED_IN in one transaction
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn check_in_by_right_qr_issues_booking_ticket_and_marks_checked_in() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
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
        5,
        1,
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B261012-0001",
        &customer_id,
        slot_id,
        "CONFIRMED",
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
        .expect("check-in with right QR succeeds");

    assert_eq!(booking.status, BookingStatus::CheckedIn);
    assert_eq!(ticket.service_type, "BOOKING");
    assert!(
        ticket.display_number.starts_with('B'),
        "booking display number must start with B, got {}",
        ticket.display_number
    );

    let (status, checked_in_at, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(status, "CHECKED_IN");
    assert!(checked_in_at.is_some(), "checked_in_at must be recorded");

    let events = h.count_booking_events(&booking_id, "CHECK_IN").await;
    assert_eq!(events, 1, "exactly one CHECK_IN event recorded");

    let ticket_row = h.get_ticket_for_booking(&booking_id).await;
    assert!(ticket_row.is_some(), "queue ticket exists for booking");
    let (t_id, t_status, t_disp, _) = ticket_row.unwrap();
    assert_eq!(t_id, ticket.id);
    assert_eq!(t_status, "WAITING");
    assert_eq!(t_disp, ticket.display_number);

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 2. Identity check by teller issues BOOKING ticket and marks booking CHECKED_IN
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn check_in_identity_checked_issues_booking_ticket_and_marks_checked_in() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (_, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Bob").await;
    h.seed_slot(
        slot_id,
        service_date,
        now - Duration::minutes(2),
        now + Duration::minutes(28),
        5,
        1,
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B261012-0002",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash,
    )
    .await;

    let (booking, ticket) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::IdentityChecked,
            TELLER_ID,
            now,
        )
        .await
        .expect("check-in by identity verification succeeds");

    assert_eq!(booking.status, BookingStatus::CheckedIn);
    assert_eq!(ticket.service_type, "BOOKING");
    assert!(ticket.display_number.starts_with('B'));

    let (status, checked_in_at, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(status, "CHECKED_IN");
    assert!(checked_in_at.is_some());

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 3. Wrong QR token returns qrMismatch, booking stays CONFIRMED, no ticket
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn check_in_wrong_qr_refuses_with_qr_mismatch() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (_, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Charlie").await;
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
        "B261012-0003",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash,
    )
    .await;

    let err = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken("wrong-qr-token".to_string()),
            TELLER_ID,
            now,
        )
        .await
        .expect_err("wrong QR must be refused");

    match err {
        CheckInError::QrMismatch => {}
        other => panic!("expected QrMismatch, got {other:?}"),
    }

    let (status, checked_in_at, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(status, "CONFIRMED", "booking must stay CONFIRMED");
    assert!(checked_in_at.is_none(), "checked_in_at must not be set");

    let ticket_row = h.get_ticket_for_booking(&booking_id).await;
    assert!(ticket_row.is_none(), "no ticket must be issued on refusal");

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 4. QR of another booking returns qrMismatch, both bookings stay CONFIRMED
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn check_in_qr_of_another_booking_refuses_with_qr_mismatch() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_1 = format!("BK-{}", unique("b1"));
    let booking_2 = format!("BK-{}", unique("b2"));
    let (_token_1, token_hash_1) = make_qr_token();
    let (token_2, token_hash_2) = make_qr_token();

    h.seed_customer(&customer_id, "Dave").await;
    h.seed_slot(
        slot_id,
        service_date,
        now - Duration::minutes(5),
        now + Duration::minutes(25),
        5,
        2,
    )
    .await;
    h.seed_booking(
        &booking_1,
        "B261012-0004",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash_1,
    )
    .await;
    h.seed_booking(
        &booking_2,
        "B261012-0005",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash_2,
    )
    .await;

    // Check in booking 1 using booking 2's QR token
    let err = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_1,
            CheckInBy::QrToken(token_2),
            TELLER_ID,
            now,
        )
        .await
        .expect_err("QR of another booking must be refused");

    match err {
        CheckInError::QrMismatch => {}
        other => panic!("expected QrMismatch, got {other:?}"),
    }

    let (status_1, _, _) = h.get_booking_status(&booking_1).await;
    let (status_2, _, _) = h.get_booking_status(&booking_2).await;
    assert_eq!(status_1, "CONFIRMED");
    assert_eq!(status_2, "CONFIRMED");

    assert!(h.get_ticket_for_booking(&booking_1).await.is_none());
    assert!(h.get_ticket_for_booking(&booking_2).await.is_none());

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 5. Closed queue returns queueClosed, booking stays CONFIRMED, no ticket
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn check_in_closed_queue_refuses_and_booking_stays_confirmed() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Eve").await;
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
        "B261012-0006",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash,
    )
    .await;

    // Disable queue at branch
    sqlx::query("UPDATE branch_service_status SET queue_open = false WHERE branch_id = $1")
        .bind(h.branch_id.as_str())
        .execute(&h.pool)
        .await
        .expect("disable queue at branch");

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
        .expect_err("closed queue must refuse check-in");

    match err {
        CheckInError::QueueClosed => {}
        other => panic!("expected QueueClosed, got {other:?}"),
    }

    let (status, checked_in_at, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(
        status, "CONFIRMED",
        "booking must stay CONFIRMED when queue is closed"
    );
    assert!(checked_in_at.is_none());

    let ticket_row = h.get_ticket_for_booking(&booking_id).await;
    assert!(
        ticket_row.is_none(),
        "no ticket issued when queue is closed"
    );

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 6. Check-in on another day returns checkInWindow, booking stays CONFIRMED
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn check_in_another_day_refuses_check_in_window() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let tomorrow_date = (now + Duration::days(1))
        .with_timezone(&Bangkok)
        .date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Frank").await;
    // Slot is tomorrow
    h.seed_slot(
        slot_id,
        tomorrow_date,
        now + Duration::days(1),
        now + Duration::days(1) + Duration::minutes(30),
        5,
        1,
    )
    .await;
    h.seed_booking(
        &booking_id,
        "B261012-0007",
        &customer_id,
        slot_id,
        "CONFIRMED",
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
        .expect_err("check-in on another day must be refused");

    match err {
        CheckInError::CheckInWindow => {}
        other => panic!("expected CheckInWindow, got {other:?}"),
    }

    let (status, _, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(status, "CONFIRMED");
    assert!(h.get_ticket_for_booking(&booking_id).await.is_none());

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 7. Check-in after start_at + grace returns checkInWindow, booking stays CONFIRMED
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn check_in_after_start_plus_grace_refuses_check_in_window() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Grace").await;
    // Slot started 20 minutes ago. Policy default grace is 15 minutes, so start_at + 15m was 5m ago.
    let start_at = now - Duration::minutes(20);
    let end_at = start_at + Duration::minutes(30);
    h.seed_slot(slot_id, service_date, start_at, end_at, 5, 1)
        .await;
    h.seed_booking(
        &booking_id,
        "B261012-0008",
        &customer_id,
        slot_id,
        "CONFIRMED",
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
        .expect_err("check-in after start + grace must be refused");

    match err {
        CheckInError::CheckInWindow => {}
        other => panic!("expected CheckInWindow, got {other:?}"),
    }

    let (status, _, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(status, "CONFIRMED");
    assert!(h.get_ticket_for_booking(&booking_id).await.is_none());

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 8. Ticket moving to SERVING via QueueDesk::start_in moves booking to IN_SERVICE
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn ticket_serving_transitions_booking_to_in_service() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Heidi").await;
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
        "B261012-0009",
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

    // Call ticket at desk
    h.desk
        .call(&h.teller_actor, &h.branch_id, &ticket.id, now)
        .await
        .expect("call ticket succeeds");

    // Start ticket at desk (CALLED -> SERVING)
    h.desk
        .start(&h.teller_actor, &h.branch_id, &ticket.id, now)
        .await
        .expect("start ticket succeeds");

    let (status, _, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(
        status, "IN_SERVICE",
        "booking must transition to IN_SERVICE when ticket is serving"
    );

    let start_events = h.count_booking_events(&booking_id, "START").await;
    assert_eq!(start_events, 1, "START booking event must be recorded");

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 9. Ticket ending NO_SHOW leaves booking CHECKED_IN; second check-in issues second ticket
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn ticket_no_show_keeps_checked_in_and_allows_second_check_in() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Ivan").await;
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
        "B261012-0010",
        &customer_id,
        slot_id,
        "CONFIRMED",
        &token_hash,
    )
    .await;

    let (_, ticket_1) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token.clone()),
            TELLER_ID,
            now,
        )
        .await
        .expect("first check-in succeeds");

    // Teller calls then skips ticket 1 (CALLED -> NO_SHOW)
    h.desk
        .call(&h.teller_actor, &h.branch_id, &ticket_1.id, now)
        .await
        .expect("call ticket succeeds");
    h.desk
        .skip(&h.teller_actor, &h.branch_id, &ticket_1.id, now)
        .await
        .expect("mark ticket no-show via skip succeeds");

    let (status, _, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(
        status, "CHECKED_IN",
        "booking must remain CHECKED_IN after ticket NO_SHOW"
    );

    // Second check-in after NO_SHOW issues a second ticket
    let (_, ticket_2) = h
        .check_in_service
        .check_in(
            &h.branch_id,
            &booking_id,
            CheckInBy::QrToken(token),
            TELLER_ID,
            now + Duration::minutes(1),
        )
        .await
        .expect("second check-in after NO_SHOW succeeds");

    assert_ne!(
        ticket_1.id, ticket_2.id,
        "second check-in must issue a distinct new ticket"
    );
    assert_eq!(ticket_2.service_type, "BOOKING");

    let (status_after_2, _, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(status_after_2, "CHECKED_IN");

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 10. Day close cancels CHECKED_IN booking with dayClosed
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn day_close_cancels_checked_in_booking_with_day_closed() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Judy").await;
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
        "B261012-0011",
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

    // Day close runs inside a transaction
    let mut tx = h.pool.begin().await.expect("begin transaction");
    let cancelled = h
        .day_hook
        .on_day_closed(&mut tx, &h.branch_id, now + Duration::hours(8))
        .await
        .expect("day close hook succeeds");
    tx.commit().await.expect("commit day close");

    assert!(
        cancelled.iter().any(|t| t.id == ticket.id),
        "ticket must be in cancelled list"
    );

    let (status, _, ended_reason) = h.get_booking_status(&booking_id).await;
    assert_eq!(
        status, "CANCELLED",
        "booking must be CANCELLED on day close"
    );
    assert_eq!(
        ended_reason.as_deref(),
        Some("dayClosed"),
        "ended_reason must be dayClosed"
    );

    let cancel_events = h.count_booking_events(&booking_id, "CANCEL").await;
    assert_eq!(
        cancel_events, 1,
        "CANCEL event must be recorded for dayClosed"
    );

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 11. Teller cancel of CHECKED_IN booking cancels its active ticket (design §4.2)
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn teller_cancel_of_checked_in_booking_cancels_active_ticket() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Mallory").await;
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
        "B261012-0012",
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

    // Teller cancels the CHECKED_IN booking, cancelling its active ticket via cancel_active_for_booking_in
    let mut tx = h.pool.begin().await.expect("begin transaction");
    let cancelled = h
        .issuer
        .cancel_active_for_booking_in(&mut tx, &h.branch_id, &booking_id, now)
        .await
        .expect("cancel active ticket succeeds");
    tx.commit().await.expect("commit cancellation");

    assert!(
        cancelled,
        "cancel_active_for_booking_in must return true when active ticket cancelled"
    );

    let ticket_row = h.get_ticket_for_booking(&booking_id).await;
    assert!(ticket_row.is_some());
    let (t_id, t_status, _, _) = ticket_row.unwrap();
    assert_eq!(t_id, ticket.id);
    assert_eq!(t_status, "CANCELLED", "ticket must be CANCELLED");

    // Repeating cancel when no active ticket is left returns false
    let mut tx2 = h.pool.begin().await.expect("begin transaction");
    let cancelled_again = h
        .issuer
        .cancel_active_for_booking_in(&mut tx2, &h.branch_id, &booking_id, now)
        .await
        .expect("cancel when no active ticket succeeds");
    tx2.commit().await.expect("commit cancellation");

    assert!(
        !cancelled_again,
        "cancel_active_for_booking_in returns false when no active ticket exists"
    );

    h.finish().await;
}

// ------------------------------------------------------------------------------------------------
// 12. Teller cancel of CHECKED_IN booking via BookingService::cancel cancels active ticket (design §4.2)
// ------------------------------------------------------------------------------------------------
#[tokio::test]
async fn booking_cancel_by_teller_cancels_checked_in_booking_and_active_ticket() {
    let h = TestHarness::start().await;
    let now = Utc::now();
    let service_date = now.with_timezone(&Bangkok).date_naive();
    let slot_id = Uuid::new_v4();
    let customer_id = format!("CUST-{}", unique("c"));
    let booking_id = format!("BK-{}", unique("b"));
    let (token, token_hash) = make_qr_token();

    h.seed_customer(&customer_id, "Nora").await;
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
        "B261012-0013",
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

    // Cancel through BookingService::cancel as teller
    let cancelled_booking = h
        .booking_service
        .cancel(&booking_id, TELLER_ID, true, Some("customer request"), now)
        .await
        .expect("teller cancel via BookingService succeeds");

    assert_eq!(
        cancelled_booking.status,
        BookingStatus::Cancelled,
        "booking returned from cancel must be CANCELLED"
    );

    let (status, _, _) = h.get_booking_status(&booking_id).await;
    assert_eq!(status, "CANCELLED", "booking in database must be CANCELLED");

    let ticket_row = h.get_ticket_for_booking(&booking_id).await;
    assert!(ticket_row.is_some(), "ticket must exist in database");
    let (t_id, t_status, _, _) = ticket_row.unwrap();
    assert_eq!(t_id, ticket.id);
    assert_eq!(
        t_status, "CANCELLED",
        "active ticket must be CANCELLED when booking cancelled by teller"
    );

    h.finish().await;
}
