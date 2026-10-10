//! Written before the implementation (ticket T15-b, #408). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! The migration `api/migrations/<timestamp>_bookings.sql` and the three Postgres
//! implementations of T15-a's traits, design `docs/design/xchange-v3.md` §5.6.4 (*Per branch
//! everywhere*), §5.6.5 (`PgBookingRepository`, `PgMemberRepository`, `UuidBookingIds`), §5.6.6
//! (the SQL and its data-migration section) and §5.6.8 (rows marked T15-b).
//!
//! - The migration: one file named `*_bookings.sql`, after every migration on `main` when the
//!   tests were written; two tables and one index and nothing else; the columns, types and rules
//!   of §5.6.6 as written; empty on a database that has branches (no backfill); and dropping the
//!   two tables by hand (`bookings`, then `members`) needs no `CASCADE` and leaves the rest,
//!   `bookingRules` documents included (item 4).
//! - `PgBookingRepository`: every field round-trips; a booking is found by (branch, id) and a
//!   stored request by (branch, request id), never by id alone; `list` is `created_at DESC, id
//!   DESC`, cut to the limit; the three guarded writes answer `true` only when a row changed,
//!   `confirm_in` from `PENDING`, `cancel_in` from `PENDING`/`CONFIRMED`, `mark_used_in` from
//!   `CONFIRMED`; every write is part of the caller's transaction.
//! - `PgMemberRepository`: `upsert_in` inserts, or for the same (branch, customer) replaces
//!   `name`, `phone`, `token_hash`, `verified_at`, `verified_by` and keeps the stored id; another
//!   member's `token_hash` answers `Ok(Err(TokenTaken))`; `list` is `verified_at, id` per branch;
//!   `find`/`find_in` take the branch, so a member is never read from another branch.
//! - `UuidBookingIds`: `BK-<uuid v4>` and `MB-<uuid v4>`.
//!
//! Every repository is called through T15-a's trait object, so the signatures are T15-a's, with
//! one change T15-b makes to the trait (lane controller, 2026-10-08, on #408):
//! `MemberRepository::find(branch, id)` and `find_in(tx, branch, id)` name the branch (§5.6.4
//! *Per branch everywhere*), where T15-a had `find(id)` and `find_in(tx, id)`.
//! Constructors the tests assume, as every other `Pg*Repository` of the workspace:
//! `PgBookingRepository::new(PgPool)`, `PgMemberRepository::new(PgPool)`; `UuidBookingIds` is a
//! unit struct.
//!
//! Times are fixed instants (`common::at`), never a clock: an order by `created_at` or
//! `verified_at` is checked on distinct and on equal instants, with the id as the tie-break.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Amended for #758 (RE-4d) by the ticket author (claude/e5ab6c), `docs/design/blueprint/rates.md`
//! §4.9 R1 and R6: a booking holds its `unit_size` and the line it was created at (the bookings
//! here are unit 1, written with no line, as before RE-4); `confirm_in` takes no rate, because
//! confirm never rewrites the rate stored at create (`confirm_in_changes_only_a_pending_booking`).

// `common` re-exports the customer types its own tests name; this file names none of them.
#[allow(unused_imports)]
mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use common::{MAIN, TestDb, at, branch};
use pos_error::AppResult;
use sqlx::{AssertSqlSafe, Postgres, Transaction};
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use xc_exchange::domain::booking::entities::booking::Booking;
use xc_exchange::domain::booking::entities::booking_steps::StoredRequest;
use xc_exchange::domain::booking::entities::member::Member;
use xc_exchange::domain::booking::repositories::booking_ids::BookingIds;
use xc_exchange::domain::booking::repositories::booking_repository::BookingRepository;
use xc_exchange::domain::booking::repositories::member_repository::{MemberRepository, TokenTaken};
use xc_exchange::domain::booking::value_objects::booking_input::token_hash;
use xc_exchange::domain::booking::value_objects::booking_status::BookingStatus;
use xc_exchange::infrastructure::booking::booking_ids::UuidBookingIds;
use xc_exchange::infrastructure::booking::booking_repository::PgBookingRepository;
use xc_exchange::infrastructure::booking::member_repository::PgMemberRepository;

const AIRPORT: &str = "AIRPORT";
/// The last migration on `main` when these tests were written (`86beca4`).
const LAST_BEFORE: &str = "20261008120000_device_agents.sql";

// ── values ───────────────────────────────────────────────────────────────────

fn money(text: &str) -> Money {
    Money::parse(text).expect("an amount")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a rate")
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a currency")
}

/// A non-member's booking as `create` stores it: CONFIRMED for 30 minutes.
fn confirmed(id: &str, branch_id: &str, created: i64) -> Booking {
    Booking {
        id: id.to_string(),
        branch_id: branch(branch_id),
        name: "สมชาย ใจดี".to_string(),
        phone: "+66 (81) 234-5678".to_string(),
        member_id: None,
        side: Side::Sell,
        currency: code("USD"),
        amount: money("1000.50"),
        rate: rate("32.6"),
        unit_size: UnitSize::ONE,
        published_rate_line_id: None,
        status: BookingStatus::Confirmed,
        created_at: at(created),
        created_by: "owner".to_string(),
        expires_at: Some(at(created + 1800)),
        confirmed_at: None,
        confirmed_by: None,
        submission_id: None,
        transaction_ref: None,
    }
}

/// A member's booking as `create` stores it: PENDING, no expiry.
fn pending(id: &str, branch_id: &str, member_id: &str, created: i64) -> Booking {
    Booking {
        id: id.to_string(),
        branch_id: branch(branch_id),
        name: "Jane Doe".to_string(),
        phone: "0812345678".to_string(),
        member_id: Some(member_id.to_string()),
        side: Side::Buy,
        currency: code("EUR"),
        amount: money("250"),
        rate: rate("35.1234"),
        unit_size: UnitSize::ONE,
        published_rate_line_id: None,
        status: BookingStatus::Pending,
        created_at: at(created),
        created_by: "FRONT-1".to_string(),
        expires_at: None,
        confirmed_at: None,
        confirmed_by: None,
        submission_id: None,
        transaction_ref: None,
    }
}

fn member(id: &str, branch_id: &str, customer_id: &str, secret: &str, verified: i64) -> Member {
    Member {
        id: id.to_string(),
        branch_id: branch(branch_id),
        customer_id: customer_id.to_string(),
        name: format!("Customer {customer_id}"),
        phone: "0899999999".to_string(),
        token_hash: token_hash(secret),
        verified_at: at(verified),
        verified_by: "owner".to_string(),
    }
}

fn ids(bookings: &[Booking]) -> Vec<&str> {
    bookings.iter().map(|b| b.id.as_str()).collect()
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Shop {
    db: TestDb,
    bookings: Arc<dyn BookingRepository>,
    members: Arc<dyn MemberRepository>,
}

impl Shop {
    /// `MAIN` and `AIRPORT`, three live customers in `MAIN` and one in `AIRPORT`.
    async fn open() -> Shop {
        let db = TestDb::start().await;
        db.seed_branch(AIRPORT, "Airport").await;
        for (id, branch_id) in [
            ("C-MAIN-1", MAIN),
            ("C-MAIN-2", MAIN),
            ("C-MAIN-3", MAIN),
            ("C-AIR-1", AIRPORT),
        ] {
            seed_customer(&db, id, branch_id).await;
        }
        let bookings: Arc<dyn BookingRepository> =
            Arc::new(PgBookingRepository::new(db.pool.clone()));
        let members: Arc<dyn MemberRepository> = Arc::new(PgMemberRepository::new(db.pool.clone()));
        Shop {
            db,
            bookings,
            members,
        }
    }

    async fn begin(&self) -> Transaction<'static, Postgres> {
        self.db.pool.begin().await.expect("begin a transaction")
    }

    /// `insert_in` in a transaction of its own, committed when it succeeded.
    async fn insert(&self, booking: &Booking, request_id: &str, hash: &str) -> AppResult<()> {
        let mut tx = self.begin().await;
        let got = self
            .bookings
            .insert_in(&mut tx, booking, request_id, hash)
            .await;
        if got.is_ok() {
            tx.commit().await.expect("commit");
        }
        got
    }

    async fn stored(&self, booking: &Booking, request_id: &str) {
        self.insert(booking, request_id, &format!("hash-of-{request_id}"))
            .await
            .unwrap_or_else(|e| panic!("insert {}: {e:?}", booking.id));
    }

    async fn found(&self, branch_id: &str, id: &str) -> Option<Booking> {
        self.bookings
            .find(&branch(branch_id), id)
            .await
            .expect("find")
    }

    async fn confirm(
        &self,
        branch_id: &str,
        id: &str,
        expires_at: DateTime<Utc>,
        when: DateTime<Utc>,
        by: &str,
    ) -> bool {
        let mut tx = self.begin().await;
        let changed = self
            .bookings
            .confirm_in(&mut tx, &branch(branch_id), id, expires_at, when, by)
            .await
            .expect("confirm_in");
        tx.commit().await.expect("commit");
        changed
    }

    async fn cancel(&self, branch_id: &str, id: &str) -> bool {
        let mut tx = self.begin().await;
        let changed = self
            .bookings
            .cancel_in(&mut tx, &branch(branch_id), id)
            .await
            .expect("cancel_in");
        tx.commit().await.expect("commit");
        changed
    }

    async fn mark_used(&self, branch_id: &str, id: &str, submission: &str, doc: &str) -> bool {
        let mut tx = self.begin().await;
        let changed = self
            .bookings
            .mark_used_in(&mut tx, &branch(branch_id), id, submission, doc)
            .await
            .expect("mark_used_in");
        tx.commit().await.expect("commit");
        changed
    }

    /// `upsert_in` in a transaction of its own, committed only when the member was stored.
    async fn upsert(&self, m: &Member) -> Result<Member, TokenTaken> {
        let mut tx = self.begin().await;
        let got = self
            .members
            .upsert_in(&mut tx, m)
            .await
            .unwrap_or_else(|e| panic!("upsert_in {} answered an AppError: {e:?}", m.id));
        match got {
            Ok(stored) => {
                tx.commit().await.expect("commit");
                Ok(stored)
            }
            Err(taken) => {
                tx.rollback().await.expect("rollback");
                Err(taken)
            }
        }
    }

    async fn by_customer(&self, branch_id: &str, customer_id: &str) -> Option<Member> {
        let mut tx = self.begin().await;
        let got = self
            .members
            .by_customer_in(&mut tx, &branch(branch_id), customer_id)
            .await
            .expect("by_customer_in");
        tx.rollback().await.expect("rollback");
        got
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_string()))
            .fetch_one(&self.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

async fn seed_customer(db: &TestDb, id: &str, branch_id: &str) {
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, b.organization_id, $1, $3, 'LIVE', $4 FROM branches b WHERE b.id = $2",
    )
    .bind(id)
    .bind(branch_id)
    .bind(format!("Customer {id}"))
    .bind(at(0))
    .execute(&db.pool)
    .await
    .expect("insert into customers");
}

/// A stored DEMO exchange (a DEMO one needs no shift) that a USED booking can name.
async fn seed_transaction(db: &TestDb, branch_id: &str, document_ref: &str, customer_id: &str) {
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, \
             customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
             bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
             value_date, paid_at, recorded_at, recorded_by) \
         VALUES ($1, $2, 'DEMO', $3, 'payload-hash', $4, 'Customer', 'THA', 'SELL', 'USD', \
             1000.50, 32.6, 32616.30, 0, 0, 32616.30, 0, 'Cash', '', '', 'owner', '', '', \
             DATE '2026-10-06', $5, $5, 'owner')",
    )
    .bind(document_ref)
    .bind(branch_id)
    .bind(format!("submission-of-{document_ref}"))
    .bind(customer_id)
    .bind(at(0))
    .execute(&db.pool)
    .await
    .expect("insert into transactions");
}

async fn run(db: &TestDb, sql: &str) -> Result<(), String> {
    sqlx::query(AssertSqlSafe(sql.to_string()))
        .execute(&db.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ── the migration (§5.6.6) ───────────────────────────────────────────────────

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn migration_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(migrations())
        .expect("read api/migrations")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .collect();
    names.sort();
    names
}

fn the_file_name() -> String {
    let ours: Vec<String> = migration_names()
        .into_iter()
        .filter(|n| n.ends_with("_bookings.sql"))
        .collect();
    assert_eq!(ours.len(), 1, "exactly one `*_bookings.sql`: {ours:?}");
    ours[0].clone()
}

/// Item 3: one file, after every migration on `main` when the tests were written. Item 4: no
/// down half.
#[test]
fn the_migration_is_one_file_after_the_last_one_with_no_down_half() {
    let file = the_file_name();
    assert!(
        file.as_str() > LAST_BEFORE,
        "{file} sorts after {LAST_BEFORE}"
    );
    let names = migration_names();
    assert!(
        names
            .iter()
            .all(|n| !n.contains(".down.") && !n.contains(".up.")),
        "no reversible migration: {names:?}"
    );
}

/// Item 1: two `CREATE TABLE` and the one index — nothing that changes, fills or drops what is
/// already there.
#[test]
fn the_file_creates_two_tables_and_one_index_and_touches_nothing_else() {
    let file = the_file_name();
    let text = std::fs::read_to_string(migrations().join(&file)).expect("read the migration");
    let sql: String = text
        .lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let heads: Vec<String> = sql
        .split(';')
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .map(|s| s.split(" (").next().unwrap_or("").to_uppercase())
        .collect();
    assert_eq!(
        heads,
        vec![
            "CREATE TABLE MEMBERS".to_string(),
            "CREATE TABLE BOOKINGS".to_string(),
            "CREATE INDEX BOOKINGS_BRANCH_CREATED_IDX ON BOOKINGS".to_string(),
        ],
        "{file}"
    );
}

/// Item 2: no backfill — on a database with two branches the two tables are empty, and no
/// `bookingRules` document exists.
#[tokio::test]
async fn the_tables_start_empty_on_a_database_with_branches() {
    let shop = Shop::open().await;
    assert!(shop.count("SELECT count(*) FROM branches").await >= 2);
    assert_eq!(shop.count("SELECT count(*) FROM bookings").await, 0);
    assert_eq!(shop.count("SELECT count(*) FROM members").await, 0);
    assert_eq!(
        shop.count("SELECT count(*) FROM branch_settings WHERE name = 'bookingRules'")
            .await,
        0
    );
    shop.finish().await;
}

/// (column, type, nullable) of a table of the test's schema, in column order.
async fn columns(db: &TestDb, table: &str) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(&db.pool)
    .await
    .expect("read information_schema")
}

fn col(name: &str, kind: &str, nullable: &str) -> (String, String, String) {
    (name.to_string(), kind.to_string(), nullable.to_string())
}

const TZ: &str = "timestamp with time zone";

/// (column, character_maximum_length, numeric_precision, numeric_scale)
type Size = (String, Option<i32>, Option<i32>, Option<i32>);

#[tokio::test]
async fn the_columns_are_the_designs() {
    let shop = Shop::open().await;
    assert_eq!(
        columns(&shop.db, "members").await,
        vec![
            col("id", "text", "NO"),
            col("branch_id", "text", "NO"),
            col("customer_id", "text", "NO"),
            col("name", "text", "NO"),
            col("phone", "text", "NO"),
            col("token_hash", "text", "NO"),
            col("verified_at", TZ, "NO"),
            col("verified_by", "text", "NO"),
            // OR-1 (#550 D-1): every table carries the organisation.
            col("organization_id", "uuid", "NO"),
        ]
    );
    assert_eq!(
        columns(&shop.db, "bookings").await,
        vec![
            col("id", "text", "NO"),
            col("branch_id", "text", "NO"),
            col("request_id", "text", "NO"),
            col("request_hash", "text", "NO"),
            col("name", "text", "NO"),
            col("phone", "text", "NO"),
            col("member_id", "text", "YES"),
            col("type", "text", "NO"),
            col("code", "character", "NO"),
            col("amount", "numeric", "NO"),
            col("rate", "numeric", "NO"),
            col("status", "text", "NO"),
            col("created_at", TZ, "NO"),
            col("created_by", "text", "NO"),
            col("expires_at", TZ, "YES"),
            col("confirmed_at", TZ, "YES"),
            col("confirmed_by", "text", "YES"),
            col("submission_id", "text", "YES"),
            col("transaction_ref", "text", "YES"),
            col("unit_size", "numeric", "NO"), // RU-1 (#595)
            // RE-4 (#601): migration 5 (`20261010125000`) links a booking to its published line
            col("published_rate_line_id", "uuid", "YES"),
            // OR-1 (#550 D-1): every table carries the organisation.
            col("organization_id", "uuid", "NO"),
        ]
    );
    let sizes: Vec<Size> = sqlx::query_as(
        "SELECT column_name::text, character_maximum_length::int4, numeric_precision::int4, \
             numeric_scale::int4 \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'bookings' \
           AND column_name IN ('code', 'amount', 'rate') ORDER BY ordinal_position",
    )
    .fetch_all(&shop.db.pool)
    .await
    .expect("read information_schema");
    assert_eq!(
        sizes,
        vec![
            ("code".to_string(), Some(3), None, None),
            ("amount".to_string(), None, Some(20), Some(2)),
            ("rate".to_string(), None, Some(20), Some(8)), // RU-1 (#595): rates at scale 8
        ]
    );
    let index: Option<String> = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes \
         WHERE schemaname = current_schema() AND indexname = 'bookings_branch_created_idx'",
    )
    .fetch_optional(&shop.db.pool)
    .await
    .expect("read pg_indexes");
    let index = index.expect("the index bookings_branch_created_idx");
    assert!(
        index.ends_with("bookings USING btree (branch_id, created_at DESC, id DESC)"),
        "{index}"
    );
    shop.finish().await;
}

/// One row of `bookings`, written in SQL so the table's own rules are what answers.
#[derive(Clone)]
struct Row {
    id: String,
    branch: &'static str,
    request_id: String,
    member: Option<&'static str>,
    kind: &'static str,
    code: &'static str,
    amount: &'static str,
    rate: &'static str,
    status: &'static str,
    expires: bool,
    submission: Option<&'static str>,
    transaction_ref: Option<&'static str>,
}

fn quoted(v: Option<&str>) -> String {
    v.map_or_else(|| "NULL".to_string(), |s| format!("'{s}'"))
}

impl Row {
    /// A valid CONFIRMED non-member booking of `MAIN`.
    fn valid(n: u32) -> Row {
        Row {
            id: format!("BK-row-{n}"),
            branch: MAIN,
            request_id: format!("req-row-{n}"),
            member: None,
            kind: "SELL",
            code: "USD",
            amount: "100.00",
            rate: "32.6000",
            status: "CONFIRMED",
            expires: true,
            submission: None,
            transaction_ref: None,
        }
    }

    fn sql(&self) -> String {
        format!(
            "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, member_id, \
                 type, code, amount, rate, status, created_at, created_by, expires_at, \
                 submission_id, transaction_ref) \
             VALUES ('{}', '{}', '{}', 'hash', 'Name', '0812345678', {}, '{}', '{}', {}, {}, \
                 '{}', now(), 'owner', {}, {}, {})",
            self.id,
            self.branch,
            self.request_id,
            quoted(self.member),
            self.kind,
            self.code,
            self.amount,
            self.rate,
            self.status,
            if self.expires {
                "now() + interval '30 minutes'"
            } else {
                "NULL"
            },
            quoted(self.submission),
            quoted(self.transaction_ref),
        )
    }
}

async fn seed_member_row(db: &TestDb, id: &str, branch_id: &str, customer_id: &str, hash: &str) {
    run(
        db,
        &format!(
            "INSERT INTO members VALUES ('{id}', '{branch_id}', '{customer_id}', 'Name', \
             '0812345678', '{hash}', now(), 'owner')"
        ),
    )
    .await
    .unwrap_or_else(|e| panic!("member {id}: {e}"));
}

#[tokio::test]
async fn the_members_table_keeps_its_rules() {
    let shop = Shop::open().await;
    let db = &shop.db;
    let row = |id: &str, branch_id: &str, customer: &str, hash: &str| {
        format!(
            "INSERT INTO members VALUES ('{id}', '{branch_id}', '{customer}', 'Name', \
             '0812345678', '{hash}', now(), 'owner')"
        )
    };
    assert_eq!(run(db, &row("MB-1", MAIN, "C-MAIN-1", "h1")).await, Ok(()));
    assert!(
        run(db, &row("MB-1", MAIN, "C-MAIN-2", "h2")).await.is_err(),
        "the id is the key"
    );
    assert!(
        run(db, &row("MB-2", MAIN, "C-MAIN-2", "h1")).await.is_err(),
        "token_hash is unique"
    );
    assert!(
        run(db, &row("MB-3", AIRPORT, "C-AIR-1", "h1"))
            .await
            .is_err(),
        "token_hash is unique across branches"
    );
    assert!(
        run(db, &row("MB-4", MAIN, "C-MAIN-1", "h4")).await.is_err(),
        "one member per (branch, customer)"
    );
    assert!(
        run(db, &row("MB-5", MAIN, "C-NOBODY", "h5")).await.is_err(),
        "a known customer"
    );
    assert!(
        run(db, &row("MB-6", "NOWHERE", "C-MAIN-2", "h6"))
            .await
            .is_err(),
        "a known branch"
    );
    assert_eq!(run(db, &row("MB-7", MAIN, "C-MAIN-2", "h7")).await, Ok(()));
    assert_eq!(shop.count("SELECT count(*) FROM members").await, 2);
    shop.finish().await;
}

#[tokio::test]
async fn the_bookings_table_keeps_its_rules() {
    let shop = Shop::open().await;
    let db = &shop.db;
    seed_member_row(db, "MB-1", MAIN, "C-MAIN-1", "h1").await;

    assert_eq!(run(db, &Row::valid(1).sql()).await, Ok(()));
    let same_id = Row {
        request_id: "req-other".to_string(),
        ..Row::valid(1)
    };
    assert!(run(db, &same_id.sql()).await.is_err(), "the id is the key");
    let same_request = Row {
        id: "BK-row-1b".to_string(),
        ..Row::valid(1)
    };
    assert!(
        run(db, &same_request.sql()).await.is_err(),
        "UNIQUE (branch_id, request_id)"
    );
    let same_request_elsewhere = Row {
        id: "BK-row-1c".to_string(),
        branch: AIRPORT,
        ..Row::valid(1)
    };
    assert_eq!(
        run(db, &same_request_elsewhere.sql()).await,
        Ok(()),
        "a request id is per branch"
    );

    let refused: Vec<(&str, Row)> = vec![
        (
            "a known branch",
            Row {
                branch: "NOWHERE",
                ..Row::valid(2)
            },
        ),
        (
            "type BUY or SELL",
            Row {
                kind: "HOLD",
                ..Row::valid(3)
            },
        ),
        (
            "a known currency",
            Row {
                code: "XXX",
                ..Row::valid(4)
            },
        ),
        (
            "amount > 0",
            Row {
                amount: "0",
                ..Row::valid(5)
            },
        ),
        (
            "rate > 0",
            Row {
                rate: "0",
                ..Row::valid(6)
            },
        ),
        (
            "EXPIRED is never stored",
            Row {
                status: "EXPIRED",
                ..Row::valid(7)
            },
        ),
        (
            "a known member",
            Row {
                member: Some("MB-nobody"),
                ..Row::valid(8)
            },
        ),
    ];
    for (rule, row) in refused {
        assert!(run(db, &row.sql()).await.is_err(), "{rule}");
    }
    for status in ["CONFIRMED", "CANCELLED"] {
        let row = Row {
            status,
            member: Some("MB-1"),
            ..Row::valid(9)
        };
        assert_eq!(run(db, &row.sql()).await, Ok(()), "{status} with a member");
        run(db, "DELETE FROM bookings WHERE id = 'BK-row-9'")
            .await
            .expect("clean up");
    }
    assert_eq!(
        shop.count("SELECT count(*) FROM bookings").await,
        2,
        "only the two valid rows"
    );
    shop.finish().await;
}

/// §5.6.8: "confirm a non-member booking still PENDING — impossible: the table's CHECK forbids
/// PENDING without a member". Also a PENDING booking has no expiry yet.
#[tokio::test]
async fn pending_needs_a_member() {
    let shop = Shop::open().await;
    let db = &shop.db;
    seed_member_row(db, "MB-1", MAIN, "C-MAIN-1", "h1").await;

    let no_member = Row {
        status: "PENDING",
        expires: false,
        ..Row::valid(1)
    };
    assert!(
        run(db, &no_member.sql()).await.is_err(),
        "PENDING without a member"
    );
    let with_expiry = Row {
        status: "PENDING",
        member: Some("MB-1"),
        expires: true,
        ..Row::valid(2)
    };
    assert!(
        run(db, &with_expiry.sql()).await.is_err(),
        "PENDING with an expiry"
    );
    let good = Row {
        status: "PENDING",
        member: Some("MB-1"),
        expires: false,
        ..Row::valid(3)
    };
    assert_eq!(run(db, &good.sql()).await, Ok(()));

    // The same rule reached through the repository: the insert fails and stores nothing.
    let mut bad = pending("BK-repo", MAIN, "MB-1", 0);
    bad.member_id = None;
    assert!(
        shop.insert(&bad, "req-repo", "hash").await.is_err(),
        "insert_in of a PENDING booking without a member"
    );
    assert_eq!(shop.found(MAIN, "BK-repo").await, None);
    shop.finish().await;
}

#[tokio::test]
async fn confirmed_needs_an_expiry_and_used_needs_its_save() {
    let shop = Shop::open().await;
    let db = &shop.db;
    seed_transaction(db, MAIN, "XC-DOC-1", "C-MAIN-1").await;

    let no_expiry = Row {
        expires: false,
        ..Row::valid(1)
    };
    assert!(
        run(db, &no_expiry.sql()).await.is_err(),
        "CONFIRMED without expires_at"
    );
    let used = |n: u32, submission: Option<&'static str>, doc: Option<&'static str>| Row {
        status: "USED",
        submission,
        transaction_ref: doc,
        ..Row::valid(n)
    };
    assert!(
        run(db, &used(2, Some("sub-2"), None).sql()).await.is_err(),
        "USED without transaction_ref"
    );
    assert!(
        run(db, &used(3, None, Some("XC-DOC-1")).sql())
            .await
            .is_err(),
        "USED without submission_id"
    );
    assert!(
        run(db, &used(4, Some("sub-4"), Some("XC-NOWHERE")).sql())
            .await
            .is_err(),
        "transaction_ref names a stored exchange"
    );
    let saved_but_not_used = Row {
        submission: Some("sub-5"),
        transaction_ref: Some("XC-DOC-1"),
        ..Row::valid(5)
    };
    assert!(
        run(db, &saved_but_not_used.sql()).await.is_err(),
        "a save's fields only on a USED booking"
    );
    assert_eq!(
        run(db, &used(6, Some("sub-6"), Some("XC-DOC-1")).sql()).await,
        Ok(())
    );
    assert_eq!(shop.count("SELECT count(*) FROM bookings").await, 1);
    shop.finish().await;
}

/// Item 4: dropping the two tables by hand, `bookings` first, needs no `CASCADE`; the rest of the
/// database stays as it was, and the `bookingRules` documents go only with the two `DELETE`s of
/// the design.
#[tokio::test]
async fn dropping_the_two_tables_by_hand_loses_only_them() {
    let shop = Shop::open().await;
    let db = &shop.db;
    seed_member_row(db, "MB-1", MAIN, "C-MAIN-1", "h1").await;
    let row = Row {
        status: "PENDING",
        member: Some("MB-1"),
        expires: false,
        ..Row::valid(1)
    };
    run(db, &row.sql()).await.expect("a booking");
    run(
        db,
        "INSERT INTO branch_settings VALUES ('MAIN', 'bookingRules', 1, \
         '{\"enabled\": true, \"minutes\": 30}', now(), 'owner')",
    )
    .await
    .expect("a bookingRules document");
    run(
        db,
        "INSERT INTO branch_setting_history VALUES ('MAIN', 'bookingRules', 1, 'owner', now(), \
         '{\"enabled\": true, \"minutes\": 30}', '{\"enabled\": true, \"minutes\": 30}')",
    )
    .await
    .expect("its history");
    let branches = shop.count("SELECT count(*) FROM branches").await;
    let customers = shop.count("SELECT count(*) FROM customers").await;

    assert!(
        run(db, "DROP TABLE members").await.is_err(),
        "bookings refers to members, so members goes second"
    );
    assert_eq!(run(db, "DROP TABLE bookings").await, Ok(()));
    assert_eq!(run(db, "DROP TABLE members").await, Ok(()));
    assert_eq!(shop.count("SELECT count(*) FROM branches").await, branches);
    assert_eq!(
        shop.count("SELECT count(*) FROM customers").await,
        customers
    );
    assert_eq!(
        shop.count("SELECT count(*) FROM branch_settings WHERE name = 'bookingRules'")
            .await,
        1,
        "the documents survive the drop"
    );
    assert_eq!(
        run(
            db,
            "DELETE FROM branch_setting_history WHERE name = 'bookingRules'"
        )
        .await,
        Ok(())
    );
    assert_eq!(
        run(
            db,
            "DELETE FROM branch_settings WHERE name = 'bookingRules'"
        )
        .await,
        Ok(())
    );
    assert_eq!(
        shop.count("SELECT count(*) FROM branch_settings WHERE name = 'bookingRules'")
            .await,
        0
    );
    shop.finish().await;
}

// ── PgBookingRepository ──────────────────────────────────────────────────────

#[tokio::test]
async fn a_confirmed_booking_round_trips_every_field() {
    let shop = Shop::open().await;
    let booking = confirmed("BK-1", MAIN, 0);
    shop.stored(&booking, "req-1").await;

    assert_eq!(shop.found(MAIN, "BK-1").await, Some(booking.clone()));
    let mut tx = shop.begin().await;
    let inside = shop
        .bookings
        .find_in(&mut tx, &branch(MAIN), "BK-1")
        .await
        .expect("find_in");
    tx.rollback().await.expect("rollback");
    assert_eq!(inside, Some(booking));
    assert_eq!(shop.found(MAIN, "BK-2").await, None, "an unknown id");
    shop.finish().await;
}

#[tokio::test]
async fn a_member_booking_and_a_used_booking_round_trip() {
    let shop = Shop::open().await;
    shop.upsert(&member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0))
        .await
        .expect("a member");
    seed_transaction(&shop.db, MAIN, "XC-DOC-1", "C-MAIN-1").await;

    let waiting = pending("BK-P", MAIN, "MB-1", 1);
    shop.stored(&waiting, "req-p").await;
    assert_eq!(shop.found(MAIN, "BK-P").await, Some(waiting));

    let used = Booking {
        member_id: Some("MB-1".to_string()),
        status: BookingStatus::Used,
        confirmed_at: Some(at(5)),
        confirmed_by: Some("MANAGER-1".to_string()),
        submission_id: Some("sub-1".to_string()),
        transaction_ref: Some("XC-DOC-1".to_string()),
        ..confirmed("BK-U", MAIN, 2)
    };
    shop.stored(&used, "req-u").await;
    assert_eq!(shop.found(MAIN, "BK-U").await, Some(used));
    shop.finish().await;
}

#[tokio::test]
async fn find_request_in_answers_the_stored_booking_and_its_hash() {
    let shop = Shop::open().await;
    let booking = confirmed("BK-1", MAIN, 0);
    shop.insert(&booking, "req-1", "a1b2c3")
        .await
        .expect("insert");

    let mut tx = shop.begin().await;
    let here = shop
        .bookings
        .find_request_in(&mut tx, &branch(MAIN), "req-1")
        .await
        .expect("find_request_in");
    let elsewhere = shop
        .bookings
        .find_request_in(&mut tx, &branch(AIRPORT), "req-1")
        .await
        .expect("find_request_in");
    let unknown = shop
        .bookings
        .find_request_in(&mut tx, &branch(MAIN), "req-2")
        .await
        .expect("find_request_in");
    let by_booking_id = shop
        .bookings
        .find_request_in(&mut tx, &branch(MAIN), "BK-1")
        .await
        .expect("find_request_in");
    tx.rollback().await.expect("rollback");

    assert_eq!(
        here,
        Some(StoredRequest {
            booking,
            hash: "a1b2c3".to_string()
        })
    );
    assert_eq!(elsewhere, None, "a request id of another branch");
    assert_eq!(unknown, None);
    assert_eq!(
        by_booking_id, None,
        "found by request id, not by booking id"
    );
    shop.finish().await;
}

#[tokio::test]
async fn the_same_request_id_is_one_booking_per_branch() {
    let shop = Shop::open().await;
    shop.stored(&confirmed("BK-1", MAIN, 0), "req-1").await;
    shop.stored(&confirmed("BK-2", AIRPORT, 0), "req-1").await;
    assert!(
        shop.insert(&confirmed("BK-3", MAIN, 1), "req-1", "other")
            .await
            .is_err(),
        "a second booking for one request id of a branch"
    );
    assert_eq!(shop.count("SELECT count(*) FROM bookings").await, 2);
    shop.finish().await;
}

/// §5.6.4 *Per branch everywhere*: an id read or written with another branch is not found and
/// changes nothing.
#[tokio::test]
async fn a_booking_is_read_and_written_only_with_its_own_branch() {
    let shop = Shop::open().await;
    shop.upsert(&member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0))
        .await
        .expect("a member");
    seed_transaction(&shop.db, MAIN, "XC-DOC-1", "C-MAIN-1").await;
    let open = confirmed("BK-C", MAIN, 0);
    let waiting = pending("BK-P", MAIN, "MB-1", 1);
    shop.stored(&open, "req-c").await;
    shop.stored(&waiting, "req-p").await;

    assert_eq!(shop.found(AIRPORT, "BK-C").await, None);
    let mut tx = shop.begin().await;
    let inside = shop
        .bookings
        .find_in(&mut tx, &branch(AIRPORT), "BK-C")
        .await
        .expect("find_in");
    tx.rollback().await.expect("rollback");
    assert_eq!(inside, None);
    assert!(
        shop.bookings
            .list(&branch(AIRPORT), 10)
            .await
            .expect("list")
            .is_empty()
    );

    assert!(
        !shop
            .confirm(AIRPORT, "BK-P", at(900), at(60), "staff")
            .await
    );
    assert!(!shop.cancel(AIRPORT, "BK-C").await);
    assert!(!shop.cancel(AIRPORT, "BK-P").await);
    assert!(!shop.mark_used(AIRPORT, "BK-C", "sub-1", "XC-DOC-1").await);

    assert_eq!(shop.found(MAIN, "BK-C").await, Some(open));
    assert_eq!(shop.found(MAIN, "BK-P").await, Some(waiting));
    shop.finish().await;
}

#[tokio::test]
async fn list_is_newest_first_then_by_id_desc_and_cut_to_the_limit() {
    let shop = Shop::open().await;
    // Inserted in an order that is neither the answer nor its reverse.
    shop.stored(&confirmed("BK-b", MAIN, 20), "req-b").await;
    shop.stored(&confirmed("BK-a", MAIN, 0), "req-a").await;
    shop.stored(&confirmed("BK-c", MAIN, 20), "req-c").await;
    shop.stored(&confirmed("BK-z", AIRPORT, 30), "req-z").await;
    shop.stored(&confirmed("BK-d", MAIN, 10), "req-d").await;

    let all = shop.bookings.list(&branch(MAIN), 10).await.expect("list");
    assert_eq!(ids(&all), vec!["BK-c", "BK-b", "BK-d", "BK-a"]);
    assert_eq!(
        all[2],
        confirmed("BK-d", MAIN, 10),
        "every field, as stored"
    );
    let two = shop.bookings.list(&branch(MAIN), 2).await.expect("list");
    assert_eq!(ids(&two), vec!["BK-c", "BK-b"]);
    let airport = shop
        .bookings
        .list(&branch(AIRPORT), 10)
        .await
        .expect("list");
    assert_eq!(ids(&airport), vec!["BK-z"]);
    shop.finish().await;
}

/// `confirm_in` changes a `PENDING` booking only (§5.6.3 confirm rule 7).
///
/// Amended for #758 (RE-4d, rates §4.9 R6): `confirm_in` took a rate and wrote it (32.75 over the
/// stored 35.1234). Confirm never rewrites the rate, the unit or the line stored at create, so it
/// takes no rate and the confirmed row keeps 35.1234.
#[tokio::test]
async fn confirm_in_changes_only_a_pending_booking() {
    let shop = Shop::open().await;
    shop.upsert(&member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0))
        .await
        .expect("a member");
    let waiting = pending("BK-P", MAIN, "MB-1", 1);
    let open = confirmed("BK-C", MAIN, 2);
    let gone = pending("BK-X", MAIN, "MB-1", 3);
    shop.stored(&waiting, "req-p").await;
    shop.stored(&open, "req-c").await;
    shop.stored(&gone, "req-x").await;
    assert!(shop.cancel(MAIN, "BK-X").await);

    assert!(shop.confirm(MAIN, "BK-P", at(600), at(60), "FRONT-2").await);
    let now_confirmed = Booking {
        status: BookingStatus::Confirmed,
        expires_at: Some(at(600)),
        confirmed_at: Some(at(60)),
        confirmed_by: Some("FRONT-2".to_string()),
        ..waiting
    };
    assert_eq!(shop.found(MAIN, "BK-P").await, Some(now_confirmed.clone()));

    assert!(
        !shop
            .confirm(MAIN, "BK-P", at(1200), at(120), "FRONT-3")
            .await,
        "a second confirm"
    );
    assert_eq!(shop.found(MAIN, "BK-P").await, Some(now_confirmed));
    assert!(
        !shop
            .confirm(MAIN, "BK-C", at(1200), at(120), "FRONT-3")
            .await,
        "a CONFIRMED non-member booking"
    );
    assert_eq!(shop.found(MAIN, "BK-C").await, Some(open));
    assert!(
        !shop
            .confirm(MAIN, "BK-X", at(1200), at(120), "FRONT-3")
            .await,
        "a CANCELLED booking"
    );
    assert_eq!(
        shop.found(MAIN, "BK-X").await.map(|b| b.status),
        Some(BookingStatus::Cancelled)
    );
    assert!(
        !shop
            .confirm(MAIN, "BK-nobody", at(1200), at(120), "FRONT-3")
            .await
    );
    shop.finish().await;
}

/// `cancel_in` changes a `PENDING` or `CONFIRMED` booking only (v2 `server.mjs:7251-7255`).
#[tokio::test]
async fn cancel_in_changes_only_a_pending_or_confirmed_booking() {
    let shop = Shop::open().await;
    shop.upsert(&member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0))
        .await
        .expect("a member");
    seed_transaction(&shop.db, MAIN, "XC-DOC-1", "C-MAIN-1").await;
    let waiting = pending("BK-P", MAIN, "MB-1", 1);
    let open = confirmed("BK-C", MAIN, 2);
    let used = Booking {
        status: BookingStatus::Used,
        submission_id: Some("sub-1".to_string()),
        transaction_ref: Some("XC-DOC-1".to_string()),
        ..confirmed("BK-U", MAIN, 3)
    };
    shop.stored(&waiting, "req-p").await;
    shop.stored(&open, "req-c").await;
    shop.stored(&used, "req-u").await;

    assert!(shop.cancel(MAIN, "BK-P").await);
    assert_eq!(
        shop.found(MAIN, "BK-P").await,
        Some(Booking {
            status: BookingStatus::Cancelled,
            ..waiting
        })
    );
    assert!(shop.cancel(MAIN, "BK-C").await);
    let cancelled = Booking {
        status: BookingStatus::Cancelled,
        ..open
    };
    assert_eq!(shop.found(MAIN, "BK-C").await, Some(cancelled.clone()));
    assert!(!shop.cancel(MAIN, "BK-C").await, "cancel after cancel");
    assert_eq!(shop.found(MAIN, "BK-C").await, Some(cancelled));
    assert!(!shop.cancel(MAIN, "BK-U").await, "a USED booking");
    assert_eq!(shop.found(MAIN, "BK-U").await, Some(used));
    assert!(!shop.cancel(MAIN, "BK-nobody").await);
    shop.finish().await;
}

/// `mark_used_in` changes a `CONFIRMED` booking only (v2 `server.mjs:7071`).
#[tokio::test]
async fn mark_used_in_changes_only_a_confirmed_booking() {
    let shop = Shop::open().await;
    shop.upsert(&member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0))
        .await
        .expect("a member");
    seed_transaction(&shop.db, MAIN, "XC-DOC-1", "C-MAIN-1").await;
    seed_transaction(&shop.db, MAIN, "XC-DOC-2", "C-MAIN-1").await;
    let open = confirmed("BK-C", MAIN, 0);
    let waiting = pending("BK-P", MAIN, "MB-1", 1);
    let gone = confirmed("BK-X", MAIN, 2);
    shop.stored(&open, "req-c").await;
    shop.stored(&waiting, "req-p").await;
    shop.stored(&gone, "req-x").await;
    assert!(shop.cancel(MAIN, "BK-X").await);

    assert!(shop.mark_used(MAIN, "BK-C", "sub-1", "XC-DOC-1").await);
    let used = Booking {
        status: BookingStatus::Used,
        submission_id: Some("sub-1".to_string()),
        transaction_ref: Some("XC-DOC-1".to_string()),
        ..open
    };
    assert_eq!(shop.found(MAIN, "BK-C").await, Some(used.clone()));
    assert!(
        !shop.mark_used(MAIN, "BK-C", "sub-2", "XC-DOC-2").await,
        "used twice"
    );
    assert_eq!(shop.found(MAIN, "BK-C").await, Some(used));
    assert!(
        !shop.mark_used(MAIN, "BK-P", "sub-2", "XC-DOC-2").await,
        "a PENDING booking"
    );
    assert_eq!(shop.found(MAIN, "BK-P").await, Some(waiting));
    assert!(
        !shop.mark_used(MAIN, "BK-X", "sub-2", "XC-DOC-2").await,
        "a CANCELLED booking"
    );
    assert_eq!(
        shop.found(MAIN, "BK-X").await.map(|b| b.status),
        Some(BookingStatus::Cancelled)
    );
    shop.finish().await;
}

/// Every write is part of the caller's transaction: rolled back, it never happened.
#[tokio::test]
async fn writes_are_part_of_the_callers_transaction() {
    let shop = Shop::open().await;
    seed_transaction(&shop.db, MAIN, "XC-DOC-1", "C-MAIN-1").await;
    let open = confirmed("BK-C", MAIN, 0);
    shop.stored(&open, "req-c").await;

    let mut tx = shop.begin().await;
    shop.bookings
        .insert_in(&mut tx, &confirmed("BK-N", MAIN, 1), "req-n", "h")
        .await
        .expect("insert_in");
    assert!(
        shop.bookings
            .find_in(&mut tx, &branch(MAIN), "BK-N")
            .await
            .expect("find_in")
            .is_some(),
        "find_in reads the caller's transaction"
    );
    assert!(
        shop.bookings
            .mark_used_in(&mut tx, &branch(MAIN), "BK-C", "sub-1", "XC-DOC-1")
            .await
            .expect("mark_used_in")
    );
    tx.rollback().await.expect("rollback");
    assert_eq!(shop.found(MAIN, "BK-N").await, None);
    assert_eq!(shop.found(MAIN, "BK-C").await, Some(open.clone()));

    let mut tx = shop.begin().await;
    assert!(
        shop.bookings
            .cancel_in(&mut tx, &branch(MAIN), "BK-C")
            .await
            .expect("cancel_in")
    );
    tx.rollback().await.expect("rollback");
    assert_eq!(shop.found(MAIN, "BK-C").await, Some(open));

    let mut tx = shop.begin().await;
    let stored = shop
        .members
        .upsert_in(&mut tx, &member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0))
        .await
        .expect("upsert_in");
    assert!(stored.is_ok());
    tx.rollback().await.expect("rollback");
    assert_eq!(
        shop.members
            .find(&branch(MAIN), "MB-1")
            .await
            .expect("find"),
        None
    );
    shop.finish().await;
}

// ── PgMemberRepository ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_member_round_trips_and_is_found_by_its_customer_in_its_branch() {
    let shop = Shop::open().await;
    let m = member("MB-1", MAIN, "C-MAIN-1", "secret-1", 7);
    assert_eq!(shop.upsert(&m).await, Ok(m.clone()));

    assert_eq!(
        shop.members
            .find(&branch(MAIN), "MB-1")
            .await
            .expect("find"),
        Some(m.clone())
    );
    let mut tx = shop.begin().await;
    let inside = shop
        .members
        .find_in(&mut tx, &branch(MAIN), "MB-1")
        .await
        .expect("find_in");
    tx.rollback().await.expect("rollback");
    assert_eq!(inside, Some(m.clone()));
    assert_eq!(
        shop.members
            .find(&branch(MAIN), "MB-2")
            .await
            .expect("find"),
        None
    );

    assert_eq!(shop.by_customer(MAIN, "C-MAIN-1").await, Some(m));
    assert_eq!(
        shop.by_customer(AIRPORT, "C-MAIN-1").await,
        None,
        "the customer named with another branch"
    );
    assert_eq!(shop.by_customer(MAIN, "C-MAIN-2").await, None);
    shop.finish().await;
}

/// §5.6.4 *Per branch everywhere*: a member of one branch is not found from another.
#[tokio::test]
async fn a_member_is_found_only_with_its_own_branch() {
    let shop = Shop::open().await;
    let m = member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0);
    shop.upsert(&m).await.expect("a member");

    assert_eq!(
        shop.members
            .find(&branch(AIRPORT), "MB-1")
            .await
            .expect("find"),
        None
    );
    let mut tx = shop.begin().await;
    let elsewhere = shop
        .members
        .find_in(&mut tx, &branch(AIRPORT), "MB-1")
        .await
        .expect("find_in");
    let here = shop
        .members
        .find_in(&mut tx, &branch(MAIN), "MB-1")
        .await
        .expect("find_in");
    tx.rollback().await.expect("rollback");
    assert_eq!(elsewhere, None);
    assert_eq!(here, Some(m.clone()));
    assert_eq!(
        shop.members
            .find(&branch(MAIN), "MB-1")
            .await
            .expect("find"),
        Some(m)
    );
    shop.finish().await;
}

/// §5.6.8: "`member` re-issued for the same customer — same member id, new hash, old code no
/// longer matches"; its bookings still point at it.
#[tokio::test]
async fn reissue_keeps_id_replaces_hash() {
    let shop = Shop::open().await;
    let first = member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0);
    shop.upsert(&first).await.expect("the first issue");
    shop.stored(&pending("BK-P", MAIN, "MB-1", 1), "req-p")
        .await;

    // The service hands the repository a fresh id from `BookingIds` on every issue.
    let again = Member {
        id: "MB-2".to_string(),
        name: "ชื่อใหม่".to_string(),
        phone: "+66 99 000 0000".to_string(),
        token_hash: token_hash("secret-2"),
        verified_at: at(100),
        verified_by: "owner-2".to_string(),
        ..first.clone()
    };
    let kept = Member {
        id: "MB-1".to_string(),
        ..again.clone()
    };
    assert_eq!(shop.upsert(&again).await, Ok(kept.clone()));
    assert_eq!(
        shop.members
            .find(&branch(MAIN), "MB-1")
            .await
            .expect("find"),
        Some(kept.clone())
    );
    assert_eq!(
        shop.members
            .find(&branch(MAIN), "MB-2")
            .await
            .expect("find"),
        None
    );
    assert_eq!(shop.by_customer(MAIN, "C-MAIN-1").await, Some(kept.clone()));
    assert_eq!(shop.count("SELECT count(*) FROM members").await, 1);
    let old: i64 = sqlx::query_scalar("SELECT count(*) FROM members WHERE token_hash = $1")
        .bind(token_hash("secret-1"))
        .fetch_one(&shop.db.pool)
        .await
        .expect("count");
    assert_eq!(old, 0, "the old code no longer matches");
    assert_eq!(
        shop.found(MAIN, "BK-P").await.and_then(|b| b.member_id),
        Some("MB-1".to_string())
    );

    // Issued once more with the same code: still this member's own hash, not another's.
    let same_code = Member {
        id: "MB-3".to_string(),
        verified_at: at(200),
        ..again
    };
    assert_eq!(
        shop.upsert(&same_code).await,
        Ok(Member {
            id: "MB-1".to_string(),
            ..same_code
        })
    );
    shop.finish().await;
}

/// §5.6.3 `member` refusal 5: the hash of another member answers `TokenTaken` — in this branch
/// or another — and stores nothing.
#[tokio::test]
async fn another_members_token_hash_is_token_taken() {
    let shop = Shop::open().await;
    let first = member("MB-1", MAIN, "C-MAIN-1", "secret-1", 0);
    shop.upsert(&first).await.expect("the first member");

    assert_eq!(
        shop.upsert(&member("MB-2", MAIN, "C-MAIN-2", "secret-1", 1))
            .await,
        Err(TokenTaken)
    );
    assert_eq!(
        shop.upsert(&member("MB-3", AIRPORT, "C-AIR-1", "secret-1", 1))
            .await,
        Err(TokenTaken)
    );
    assert_eq!(shop.count("SELECT count(*) FROM members").await, 1);
    assert_eq!(shop.by_customer(MAIN, "C-MAIN-2").await, None);
    assert_eq!(shop.by_customer(MAIN, "C-MAIN-1").await, Some(first));

    let other = member("MB-2", MAIN, "C-MAIN-2", "secret-2", 2);
    assert_eq!(shop.upsert(&other).await, Ok(other.clone()), "its own code");
    shop.finish().await;
}

#[tokio::test]
async fn members_list_in_verified_at_then_id_order_per_branch() {
    let shop = Shop::open().await;
    // Inserted in an order that is neither the answer nor its reverse.
    for m in [
        member("MB-b", MAIN, "C-MAIN-1", "s-b", 10),
        member("MB-z", AIRPORT, "C-AIR-1", "s-z", 5),
        member("MB-c", MAIN, "C-MAIN-3", "s-c", 0),
        member("MB-a", MAIN, "C-MAIN-2", "s-a", 10),
    ] {
        shop.upsert(&m).await.expect("a member");
    }
    let main = shop.members.list(&branch(MAIN)).await.expect("list");
    assert_eq!(
        main.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["MB-c", "MB-a", "MB-b"]
    );
    assert_eq!(main[0], member("MB-c", MAIN, "C-MAIN-3", "s-c", 0));
    let airport = shop.members.list(&branch(AIRPORT)).await.expect("list");
    assert_eq!(
        airport.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["MB-z"]
    );
    shop.finish().await;
}

// ── UuidBookingIds ───────────────────────────────────────────────────────────

fn uuid_v4_after(id: &str, prefix: &str) -> uuid::Uuid {
    let rest = id
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("{id:?} starts with {prefix:?}"));
    assert_eq!(rest.len(), 36, "{id:?}: a hyphenated UUID after the prefix");
    let parsed = uuid::Uuid::parse_str(rest).unwrap_or_else(|e| panic!("{id:?}: {e}"));
    assert_eq!(parsed.get_version_num(), 4, "{id:?} is a v4 UUID");
    parsed
}

#[test]
fn ids_are_bk_and_mb_with_a_v4_uuid() {
    let ids: Box<dyn BookingIds> = Box::new(UuidBookingIds);
    let (b1, b2) = (ids.booking_id(), ids.booking_id());
    let (m1, m2) = (ids.member_id(), ids.member_id());
    assert_ne!(uuid_v4_after(&b1, "BK-"), uuid_v4_after(&b2, "BK-"));
    assert_ne!(uuid_v4_after(&m1, "MB-"), uuid_v4_after(&m2, "MB-"));
}
