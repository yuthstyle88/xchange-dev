//! BK-3 #1380 — database harness shared by booking tests.
//!
//! Every test works in a PostgreSQL schema of its own, built from every migration in `api/migrations/`.
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_booking::application::booking_service::BookingService;
use xc_booking::infrastructure::booking_repository::PgBookingRepository;
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_customer::application::customer::ContactReserve;
use xc_customer::install_pii_keys;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_kernel::pii::PiiKeys;
use xc_referral::application::capture_service::ReferralCaptureService;
use xc_referral::infrastructure::capture_repository::PgCaptureRepository;

pub const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const MUST_FINISH: Duration = Duration::from_secs(120);

static NEXT: AtomicU64 = AtomicU64::new(0);
static STARTING: Semaphore = Semaphore::const_new(4);

pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set.\n\
             The booking tests need a real PostgreSQL and do not skip. Point it at an existing \
             database whose name ends in `_test`, e.g. \
             TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
        )
    })
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

pub struct TestDb {
    pub pool: PgPool,
    pub schema: String,
}

impl TestDb {
    pub async fn start_with(connections: u32) -> Self {
        let options =
            PgConnectOptions::from_str(&url()).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("bk3");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(connections)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let db = Self { pool, schema };
        {
            let _slot = STARTING.acquire().await.expect("never closed");
            let dir = migrations_dir();
            let migrator = Migrator::new(dir.as_path())
                .await
                .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
            migrator
                .run(&db.pool)
                .await
                .unwrap_or_else(|e| panic!("every migration applies on a fresh database: {e}"));
        }
        assert!(
            db.text("SELECT to_regclass('booking_slots')::text")
                .await
                .is_some(),
            "BK-1's M1 created booking_slots"
        );
        db
    }

    pub async fn start() -> Self {
        Self::start_with(8).await
    }

    pub async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    pub async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    pub async fn count(&self, sql: &str) -> i64 {
        let (n,): (i64,) = sqlx::query_as(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        n
    }

    pub async fn text(&self, sql: &str) -> Option<String> {
        let row: Option<(Option<String>,)> = sqlx::query_as(AssertSqlSafe(sql.to_owned()))
            .fetch_optional(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        row.and_then(|(t,)| t)
    }

    pub async fn add_branch_in_org(
        &self,
        org_id: &str,
        timezone: &str,
        hours: Option<&str>,
        booking_enabled: bool,
    ) -> BranchId {
        let id = unique("BR").replace('_', "-");
        let hours = hours.map_or("NULL".to_owned(), |h| format!("'{h}'::jsonb"));
        self.exec(&format!(
            "INSERT INTO branches (id, organization_id, name, timezone, opening_hours, booking_enabled) \
             VALUES ('{id}', '{org_id}', 'Branch {id}', '{timezone}', {hours}, {booking_enabled})"
        ))
        .await;
        BranchId::new(id)
    }

    pub async fn add_branch(
        &self,
        timezone: &str,
        hours: Option<&str>,
        booking_enabled: bool,
    ) -> BranchId {
        self.add_branch_in_org(ORG_1, timezone, hours, booking_enabled)
            .await
    }

    pub async fn set_booking_enabled(&self, branch: &BranchId, enabled: bool) {
        self.exec(&format!(
            "UPDATE branches SET booking_enabled = {enabled} WHERE id = '{}'",
            branch.as_str()
        ))
        .await;
    }

    pub async fn add_policy(&self, cutoff: i32, grace: i32, expiry: i32) {
        self.exec(&format!(
            "INSERT INTO booking_policies (organization_id, cancel_cutoff_minutes, \
             no_show_grace_minutes, confirm_expiry_minutes) \
             VALUES ('{ORG_1}', {cutoff}, {grace}, {expiry}) \
             ON CONFLICT (organization_id) DO UPDATE SET \
             cancel_cutoff_minutes = EXCLUDED.cancel_cutoff_minutes, \
             no_show_grace_minutes = EXCLUDED.no_show_grace_minutes, \
             confirm_expiry_minutes = EXCLUDED.confirm_expiry_minutes"
        ))
        .await;
    }

    pub async fn add_slot_rule(
        &self,
        branch: &BranchId,
        slot_minutes: i32,
        capacity: i32,
        horizon_days: i32,
    ) {
        let org_id: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_one(&self.pool)
                .await
                .expect("branch exists");
        let user_id = self.add_user_in_org(&org_id).await;
        self.exec(&format!(
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

    pub async fn add_slot_rule_in_org(
        &self,
        org_id: &str,
        branch: &BranchId,
        slot_minutes: i32,
        capacity: i32,
        horizon_days: i32,
    ) {
        let user_id = self.add_user_in_org(org_id).await;
        self.exec(&format!(
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

    pub async fn add_user(&self) -> Uuid {
        self.add_user_in_org(ORG_1).await
    }

    pub async fn add_user_in_org(&self, org_id: &str) -> Uuid {
        let id = self
            .text(&format!(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ('{org_id}', '{}', 'ACTIVE', 'test') RETURNING id::text",
                unique("user")
            ))
            .await
            .expect("a user id");
        Uuid::parse_str(&id).expect("a uuid")
    }

    pub async fn add_slot(
        &self,
        branch: &BranchId,
        start_at: DateTime<Utc>,
        minutes: i64,
        capacity: i32,
        reserved: i32,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let end_at = start_at + ChronoDuration::minutes(minutes);
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, \
             end_at, capacity, reserved, status) \
             SELECT $1, organization_id, id, ($2 AT TIME ZONE timezone)::date, $2, $3, $4, $5, 'OPEN' \
             FROM branches WHERE id = $6",
        )
        .bind(id)
        .bind(start_at)
        .bind(end_at)
        .bind(capacity)
        .bind(reserved)
        .bind(branch.as_str())
        .execute(&self.pool)
        .await
        .expect("insert a slot");
        id
    }

    pub async fn slot_row(&self, slot: Uuid) -> (i32, i32, String) {
        sqlx::query_as("SELECT capacity, reserved, status FROM booking_slots WHERE id = $1")
            .bind(slot)
            .fetch_one(&self.pool)
            .await
            .expect("the slot row")
    }

    pub async fn add_customer_in_org(&self, org_id: &str, name: &str, phone: &str) -> String {
        let u = Uuid::new_v4();
        let id = format!("CX-{u}");
        let customer_no = format!("NO-{u}");
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at, updated_at) \
             VALUES ('{id}', '{org_id}', '{customer_no}', '{name}', 'LIVE', now(), now())"
        ))
        .await;

        let keys = self.pii_keys();
        let normalized =
            xc_kernel::pii::normalize(xc_kernel::pii::IdentityType::Phone, phone, None)
                .expect("valid phone");
        let hash = xc_kernel::pii::LookupHasher::new(&keys)
            .hash(xc_kernel::pii::IdentityType::Phone, &normalized);
        let cipher = xc_kernel::pii::PiiCipher::new(Arc::new(keys));
        let sealed = cipher.encrypt(format!("{id}PHONE").as_bytes(), phone.as_bytes());

        sqlx::query(
            "INSERT INTO customer_identities (id, organization_id, customer_id, type, \
             encrypted_value, key_id, lookup_hash, status, created_at) \
             VALUES ($1, $2, $3, 'PHONE', $4, $5, $6, 'ACTIVE', now())",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::parse_str(org_id).unwrap())
        .bind(&id)
        .bind(&sealed.bytes)
        .bind(&sealed.key_id)
        .bind(&hash[..])
        .execute(&self.pool)
        .await
        .expect("insert identity");

        id
    }

    pub async fn add_customer(&self, name: &str, phone: &str) -> String {
        self.add_customer_in_org(ORG_1, name, phone).await
    }

    pub async fn add_quote(
        &self,
        branch: &BranchId,
        side: Side,
        currency: &str,
        quantity: Decimal,
        status: &str,
    ) -> Uuid {
        let quote_id = Uuid::new_v4();
        let pub_id = Uuid::new_v4();
        let req_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();
        let now = Utc::now();
        let exp = now + ChronoDuration::minutes(15);
        let user = self.add_user().await;

        self.exec(&format!(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
             VALUES ('{req_id}', '{ORG_1}', 'RATE_PUBLICATION', '{}', '{pub_id}', 'test', \
             '1', '{{}}'::jsonb, '{user}', now(), 'APPROVED', 'k-{req_id}')",
            branch.as_str()
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, \
             approved_request_id, draft_id, published_by) \
             VALUES ('{pub_id}', '{}', 1, now(), '{req_id}', NULL, '{user}')",
            branch.as_str()
        ))
        .await;

        self.exec(&format!(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES ('{line_id}', '{pub_id}', '{currency}', 'ALL', 1.0, 34.0, 35.0, '{{}}'::jsonb)"
        ))
        .await;

        let side_str = match side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        };

        sqlx::query(
            "INSERT INTO quotes (id, organization_id, branch_id, type, channel, status, \
             publication_id, expires_at, created_by, created_at, updated_at) \
             VALUES ($1, $2, $3, 'INDICATIVE', 'BOOKING', $4, $5, $6, 'test', now(), now())",
        )
        .bind(quote_id)
        .bind(Uuid::parse_str(ORG_1).unwrap())
        .bind(branch.as_str())
        .bind(status)
        .bind(pub_id)
        .bind(exp)
        .execute(&self.pool)
        .await
        .expect("insert quote");

        sqlx::query(
            "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, \
             shop_side, currency, denomination_group, quantity, unit_size, base_rate, final_rate, \
             thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'ALL', $7, 1.0, 34.0, 34.0, 3400.0, 0, 3400.0, 1, now(), now())",
        )
        .bind(Uuid::new_v4())
        .bind(Uuid::parse_str(ORG_1).unwrap())
        .bind(quote_id)
        .bind(line_id)
        .bind(side_str)
        .bind(currency)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .expect("insert quote line");

        quote_id
    }

    pub fn pii_keys(&self) -> PiiKeys {
        PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("valid keys")
    }

    pub fn service(&self) -> BookingService {
        let keys = self.pii_keys();
        let _ = install_pii_keys(Arc::new(keys));
        let pii_keys = self.pii_keys();
        let transactor = pos_infra::transactions::Transactor::new(self.pool.clone());
        let branch_tx = BranchTx::new(transactor);
        let bookings = Arc::new(PgBookingRepository::new(self.pool.clone()));
        let slots = Arc::new(PgSlotRepository::new(self.pool.clone()));
        let contact_reserve = Arc::new(ContactReserve::new(branch_tx));
        let capture_repo = Arc::new(PgCaptureRepository::new());
        let referral_capture = Some(Arc::new(ReferralCaptureService::new(
            capture_repo,
            Arc::new(PgAuditLog::new(self.pool.clone())),
        )));

        BookingService::new(
            self.pool.clone(),
            bookings,
            slots,
            contact_reserve,
            referral_capture,
            pii_keys,
        )
    }

    pub fn slot_repo(&self) -> Arc<PgSlotRepository> {
        Arc::new(PgSlotRepository::new(self.pool.clone()))
    }

    pub fn booking_repo(&self) -> Arc<PgBookingRepository> {
        Arc::new(PgBookingRepository::new(self.pool.clone()))
    }

    pub fn contact_reserve(&self) -> Arc<ContactReserve> {
        let keys = self.pii_keys();
        let _ = install_pii_keys(Arc::new(keys));
        let transactor = pos_infra::transactions::Transactor::new(self.pool.clone());
        let branch_tx = BranchTx::new(transactor);
        Arc::new(ContactReserve::new(branch_tx))
    }
}
