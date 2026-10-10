//! FD-1 (#563 design `docs/design/blueprint/broker-seam.md` §6, §11) — the database harness and
//! the seeds of the exchange-feed tests. Written before the implementation by the ticket author.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing database whose
//! name ends in `_test`; without it every DB test FAILS, none skips. Each `TestDb` is a schema of
//! its own with `api/migrations/` applied (FD-1's migration, `20261105100000_broker_exchange_feed.sql`,
//! is the implementer's). SQL must name tables without a schema prefix.
#![allow(dead_code, unused_imports)]

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use uuid::Uuid;

pub use xc_referral::application::feed_writer::{CompletedExchange, ExchangeFeedWriter};
pub use xc_referral::domain::customer_keys::{CustomerKeys, FeedCustomerKeys};
pub use xc_referral::domain::feed_cursor::{CursorError, FeedCursor};
pub use xc_referral::domain::feed_reader::{
    ExchangeFeedReader, FeedItem, FeedPage, FeedStatus, MAX_LIMIT,
};
pub use xc_referral::infrastructure::booking_member_keys::BookingMemberKeys;
pub use xc_referral::infrastructure::feed_reader::PgExchangeFeedReader;

pub const MUST_FINISH: Duration = Duration::from_secs(30);
/// The organisation every v3 database has (`20261011100000_organizations.sql:9-10`).
pub const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// Seeded by the migrations (`20261007100000_branches_rates.sql:9`).
pub const MAIN: &str = "MAIN";
/// The member's phone the seeds use.
pub const MEMBER_PHONE: &str = "0812345678";

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// A fixed instant plus `seconds`, with microseconds, so round trips through `timestamptz` are exact.
pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_288_000 + seconds, 123_456_000)
        .single()
        .expect("a valid instant")
}

pub fn completed(transaction_id: Uuid, value_satang: i64, keys: CustomerKeys) -> CompletedExchange {
    CompletedExchange {
        transaction_id,
        organization_id: ORG.to_owned(),
        branch_id: MAIN.to_owned(),
        keys,
        value_satang,
        completed_at: at(0),
    }
}

/// One row of `broker_exchange_feed`, read back by the tests.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Row {
    pub status: String,
    pub value_satang: i64,
    pub reversed_satang: i64,
    pub phone: Option<String>,
    pub customer_sub: Option<String>,
    pub reversed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

static STARTING: Semaphore = Semaphore::const_new(6);

impl TestDb {
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The xc-referral tests need a real PostgreSQL and do \
                 not skip, e.g. TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );

        let schema = unique("fd1");
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
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        Self { pool, schema }
    }

    pub async fn begin(&self) -> Transaction<'static, Postgres> {
        self.pool.begin().await.expect("begin")
    }

    /// A customer of MAIN's organisation (the CI-4 shape, as `xc-exchange`'s tests seed it).
    pub async fn seed_customer(&self, id: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, b.organization_id, $1, $1, 'LIVE', $3 FROM branches b WHERE b.id = $2",
        )
        .bind(id)
        .bind(MAIN)
        .bind(at(0))
        .execute(&self.pool)
        .await
        .expect("insert into customers");
    }

    /// A stored DEMO exchange (a DEMO one needs no shift); answers its id and its document_ref.
    pub async fn seed_transaction(&self, customer_id: &str) -> (Uuid, String) {
        let document_ref = unique("DOC");
        let id: String = sqlx::query_scalar(
            "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, \
                 customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
                 bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
                 value_date, paid_at, recorded_at, recorded_by) \
             VALUES ($1, $2, 'DEMO', $3, 'payload-hash', $4, 'Customer', 'THA', 'SELL', 'USD', \
                 1000.50, 32.6, 32616.30, 0, 0, 32616.30, 0, 'Cash', '', '', 'owner', '', '', \
                 DATE '2026-10-06', $5, $5, 'owner') RETURNING id::text",
        )
        .bind(&document_ref)
        .bind(MAIN)
        .bind(format!("submission-of-{document_ref}"))
        .bind(customer_id)
        .bind(at(0))
        .fetch_one(&self.pool)
        .await
        .expect("insert into transactions");
        (Uuid::parse_str(&id).expect("a uuid"), document_ref)
    }

    /// A member of MAIN (verified by staff: `members.verified_at` is NOT NULL).
    pub async fn seed_member(&self, id: &str, customer_id: &str) {
        sqlx::query(
            "INSERT INTO members (id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by) \
             VALUES ($1, $2, $3, 'Name', $4, $5, now(), 'owner')",
        )
        .bind(id)
        .bind(MAIN)
        .bind(customer_id)
        .bind(MEMBER_PHONE)
        .bind(unique("hash"))
        .execute(&self.pool)
        .await
        .expect("insert into members");
    }

    /// A USED booking that became `document_ref`, made by `member_id` (or by nobody).
    pub async fn seed_used_booking(&self, member_id: Option<&str>, document_ref: &str) {
        sqlx::query(
            "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, member_id, \
                 type, code, amount, rate, status, created_at, created_by, expires_at, submission_id, \
                 transaction_ref) \
             VALUES ($1, $2, $3, 'h', 'Name', '0899999999', $4, 'SELL', 'USD', 100, 32.6, 'USED', \
                 now(), 'owner', now() + interval '1 hour', $5, $6)",
        )
        .bind(unique("BK"))
        .bind(MAIN)
        .bind(unique("req"))
        .bind(member_id)
        .bind(unique("sub"))
        .bind(document_ref)
        .execute(&self.pool)
        .await
        .expect("insert into bookings");
    }

    pub async fn row(&self, transaction_id: Uuid) -> Option<Row> {
        sqlx::query_as(
            "SELECT status, value_satang, reversed_satang, phone, customer_sub, reversed_at, updated_at \
             FROM broker_exchange_feed WHERE transaction_id = $1::uuid",
        )
        .bind(transaction_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .expect("read the feed row")
    }

    /// Record `transaction_id` as completed, in a transaction of its own.
    pub async fn record(&self, exchange: &CompletedExchange) {
        let mut tx = self.begin().await;
        ExchangeFeedWriter::record_completed(&mut tx, exchange)
            .await
            .expect("record_completed");
        tx.commit().await.expect("commit");
    }

    pub async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }
}
