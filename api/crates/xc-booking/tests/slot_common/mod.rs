//! BK-2 #1379 — the database harness shared by `slot_capacity_test.rs` and `slot_service_test.rs`.
//! Written before the implementation by the test writer (claude/dell-claude-4, on the BE lane controller
//! claude/ad05bf8d's order). Do not edit, delete or weaken; add helpers in a file of your own.
//!
//! Every test works in a PostgreSQL schema of its own, built from every migration in `api/migrations/` (BK-1's
//! M1 `*_booking_slots.sql` included). Needs `TEST_DATABASE_URL` naming an existing database whose name ends in
//! `_test`; nothing here skips.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xc_booking::application::slot_service::SlotService;
use xc_booking::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_kernel::ids::BranchId;

/// The organisation OR-1's migration creates (`20261011100000_organizations.sql`).
pub const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const MUST_FINISH: Duration = Duration::from_secs(120);

static STARTING: Semaphore = Semaphore::const_new(4);

pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
    schema: String,
}

impl TestDb {
    /// A fresh schema with every migration applied. `connections` sizes the pool (the race tests need more
    /// than one connection per racer group).
    pub async fn start_with(connections: u32) -> Self {
        let options =
            PgConnectOptions::from_str(&url()).expect("TEST_DATABASE_URL is a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("bk2");
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

    /// A branch of organisation 1 with this IANA timezone and `opening_hours` (`None` = NULL, hours not set).
    pub async fn add_branch(&self, timezone: &str, hours: Option<&str>) -> BranchId {
        let id = unique("BR").replace('_', "-");
        let hours = hours.map_or("NULL".to_owned(), |h| format!("'{h}'::jsonb"));
        self.exec(&format!(
            "INSERT INTO branches (id, name, timezone, opening_hours) \
             VALUES ('{id}', 'Branch {id}', '{timezone}', {hours})"
        ))
        .await;
        BranchId::new(id)
    }

    pub async fn set_hours(&self, branch: &BranchId, hours: &str) {
        self.exec(&format!(
            "UPDATE branches SET opening_hours = '{hours}'::jsonb WHERE id = '{}'",
            branch.as_str()
        ))
        .await;
    }

    /// An ACTIVE user of organisation 1 (`users.id`), the ADMIN who changes slot settings.
    pub async fn add_user(&self) -> Uuid {
        let id = self
            .text(&format!(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ('{ORG_1}', '{}', 'ACTIVE', 'test') RETURNING id::text",
                unique("admin")
            ))
            .await
            .expect("a user id");
        Uuid::parse_str(&id).expect("a uuid")
    }

    /// One `booking_slots` row inserted directly (not through the generator): OPEN unless `closed`.
    pub async fn add_slot(
        &self,
        branch: &BranchId,
        start_at: DateTime<Utc>,
        minutes: i64,
        capacity: i32,
        reserved: i32,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let end_at = start_at + chrono::Duration::minutes(minutes);
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, \
                 capacity, reserved, status) \
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

    /// (capacity, reserved, status) of a slot as stored.
    pub async fn slot_row(&self, slot: Uuid) -> (i32, i32, String) {
        sqlx::query_as("SELECT capacity, reserved, status FROM booking_slots WHERE id = $1")
            .bind(slot)
            .fetch_one(&self.pool)
            .await
            .expect("the slot row")
    }

    /// Every slot of the branch, ordered by `start_at`: (id, service_date, start_at, end_at, capacity, reserved,
    /// status).
    pub async fn slots(&self, branch: &BranchId) -> Vec<SlotRow> {
        sqlx::query_as(
            "SELECT id, service_date, start_at, end_at, capacity, reserved, status \
             FROM booking_slots WHERE branch_id = $1 ORDER BY start_at",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await
        .expect("the branch's slots")
    }

    pub fn repo(&self) -> Arc<PgSlotRepository> {
        Arc::new(PgSlotRepository::new(self.pool.clone()))
    }

    pub fn service(&self) -> SlotService {
        SlotService::new(self.pool.clone(), self.repo())
    }
}

pub type SlotRow = (
    Uuid,
    NaiveDate,
    DateTime<Utc>,
    DateTime<Utc>,
    i32,
    i32,
    String,
);

/// The status a returned slot carries, as the table's text.
pub fn status_text(slot: &BookingSlot) -> &'static str {
    match slot.status {
        SlotStatus::Open => "OPEN",
        SlotStatus::Closed => "CLOSED",
    }
}
