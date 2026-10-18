//! QU-A1 — the database harness shared by `queue_migration_test.rs` and `ticket_issuer_test.rs`.
//! Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or
//! weaken; add helpers in a file of your own.
//!
//! Every test works in a PostgreSQL schema of its own, built from `api/migrations/`. Needs
//! `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

#![allow(dead_code)]

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use xc_kernel::branch_tx::BranchTx;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::infrastructure::repository::PgQueueRepository;

/// The queue's migration (QU-A1 file scope). Its version must stay above main's highest
/// migration (`.github/scripts/migration-order.sh`); if the gate asks for a rename, the ticket
/// author changes this constant in a commit of their own.
pub const M1: i64 = 20261101100000;
pub const M1_FILE: &str = "20261101100000_queue.sql";
/// The organisation OR-1's migration creates (`20261011100000_organizations.sql`).
pub const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const MUST_FINISH: Duration = Duration::from_secs(60);

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
             The queue tests need a real PostgreSQL and do not skip. Point it at an existing \
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
    schema: String,
}

impl TestDb {
    /// An empty schema of its own; no migration applied yet.
    pub async fn empty() -> Self {
        let options =
            PgConnectOptions::from_str(&url()).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("qua1");
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
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Self { pool, schema }
    }

    async fn migrator(&self) -> Migrator {
        let dir = migrations_dir();
        let migrator = Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
        assert!(
            migrator.migrations.iter().any(|m| m.version == M1),
            "api/migrations has no {M1_FILE} (QU-A1 file scope)"
        );
        migrator
    }

    /// Apply every migration except M1 — the state of a database deployed before QU-A1. BK-1's
    /// `*_booking_slots.sql` re-points `queue_tickets.booking_id`, so it deploys after M1 and is held back
    /// with it (`docs/design/blueprint/booking-slots.md` §8 M1 item 3). M2 drops `v2_bookings`, which only M1 creates, so it is held back too.
    pub async fn migrate_all_but_m1(&self) {
        let _slot = STARTING.acquire().await.expect("never closed");
        let mut migrator = self.migrator().await;
        let kept: Vec<_> = migrator
            .migrations
            .iter()
            .filter(|m| {
                m.version != M1
                    && m.description != "booking slots"
                    && m.description != "retire v2 bookings"
            })
            .cloned()
            .collect();
        migrator.migrations = Cow::Owned(kept);
        migrator
            .run(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("apply every migration but M1: {e}"));
    }

    /// Apply every migration not applied yet, M1 included.
    pub async fn migrate_all(&self) -> Result<(), String> {
        let _slot = STARTING.acquire().await.expect("never closed");
        self.migrator()
            .await
            .run(&self.pool)
            .await
            .map_err(|e| e.to_string())
    }

    /// A fresh database: every migration applied in version order.
    pub async fn start() -> Self {
        let db = Self::empty().await;
        db.migrate_all()
            .await
            .unwrap_or_else(|e| panic!("every migration applies on a fresh database: {e}"));
        db
    }

    /// Drop the schema. Reached only by a test that passed.
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

    /// The SQLSTATE `sql` fails with, or `"ok"` when it succeeds.
    pub async fn outcome(&self, sql: &str) -> String {
        match sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
        {
            Ok(_) => "ok".to_owned(),
            Err(e) => match e.as_database_error().and_then(|d| d.code()) {
                Some(code) => code.into_owned(),
                None => panic!("{sql}: not a database error: {e}"),
            },
        }
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

    /// A branch of organisation 1 with this IANA timezone. Inserted after M1 it has no
    /// `branch_service_status` row; [`TestDb::open_queue`] adds one.
    pub async fn add_branch(&self, id: &str, timezone: &str) {
        self.exec(&format!(
            "INSERT INTO branches (id, name, timezone) VALUES ('{id}', 'Branch {id}', '{timezone}')"
        ))
        .await;
    }

    /// The branch's queue is open (a status row with `queue_open = true`).
    pub async fn open_queue(&self, branch: &str) {
        self.exec(&format!(
            "INSERT INTO branch_service_status (branch_id, organization_id, queue_open) \
             SELECT id, organization_id, true FROM branches WHERE id = '{branch}' \
             ON CONFLICT (branch_id) DO UPDATE SET queue_open = true"
        ))
        .await;
    }

    pub async fn close_queue(&self, branch: &str) {
        self.exec(&format!(
            "UPDATE branch_service_status SET queue_open = false WHERE branch_id = '{branch}'"
        ))
        .await;
    }

    /// A WAITING kiosk request (the shape `20261007170000_kiosk.sql` and later migrations need).
    pub async fn add_kiosk_request(&self, branch: &str, id: &str) {
        self.exec(&format!(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, side, currency, rate, \
                 foreign_amount, thb, input_amount, payment_method) \
             VALUES ('{branch}', '{id}', 'WAITING', now(), 'BUY', 'USD', 35.0000, 100.00, 3500.00, \
                 100.00, 'Cash')"
        ))
        .await;
    }

    /// A CONFIRMED Blueprint booking of `branch` (BK-1's `*_booking_slots.sql`: `queue_tickets.booking_id`
    /// names the new `bookings`), with its own customer and OPEN slot.
    pub async fn add_booking(&self, branch: &str, id: &str) {
        self.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT 'CU-{id}', organization_id, 'CU-{id}', 'Booked', 'LIVE', now() FROM branches WHERE id = '{branch}'"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, \
                 capacity, reserved, status) \
             SELECT gen_random_uuid(), organization_id, id, current_date, now() + interval '1 hour', \
                 now() + interval '90 minutes', 4, 1, 'OPEN' FROM branches WHERE id = '{branch}'"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
                 requested_currency, requested_quantity, source_channel, status, qr_token_hash, created_by) \
             SELECT '{id}', s.organization_id, s.branch_id, 'B-{id}', 'CU-{id}', s.id, 'BUY', 'USD', 100, 'DESK', \
                 'CONFIRMED', convert_to('qr-{id}', 'UTF8'), 'test' \
             FROM booking_slots s WHERE s.branch_id = '{branch}' ORDER BY s.created_at DESC LIMIT 1"
        ))
        .await;
    }

    /// A user of organisation 1 (`users.id`).
    pub async fn add_user(&self, name: &str) -> String {
        self.text(&format!(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ('{ORG_1}', '{name}', 'ACTIVE', 'test') RETURNING id::text"
        ))
        .await
        .expect("a user id")
    }

    pub fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    pub fn issuer(&self) -> TicketIssuer {
        TicketIssuer::new(
            self.branch_tx(),
            Arc::new(PgQueueRepository::new(self.pool.clone())),
        )
    }
}
