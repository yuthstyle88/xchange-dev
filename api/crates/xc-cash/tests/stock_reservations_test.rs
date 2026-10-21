//! Up-front tests for SR-1 (B05 core; design `docs/design/blueprint/stock-reservation.md` D-1 … D-8, D-15, D-16,
//! §4.2 S1–S7, S9–S12 (the store's part), S15, S20). Written by the B05 sub-controller claude/6530d490 on
//! `feat/b05-stock` before the implementation. **Do not edit, delete or weaken.**
//!
//! Until the binding quote lands (O-1 A, D-12) nothing calls `reserve_in` but these tests, so the store is reached
//! directly: each test seeds a branch, a quote with one line and an `inventory_positions` row in SQL.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test` (and the SR-0 migration in
//! `api/migrations`). Each test works in a fresh schema with every migration applied.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_cash::domain::reservation::availability::StockAvailability;
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::domain::reservation::stock_reservation::{
    ReleaseReason, ReservationId, ReservationStatus, StockReservation,
};
use xc_cash::infrastructure::stock_reservations::{PgStockReservations, active_reserved_in};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a valid decimal")
}

fn org() -> Uuid {
    Uuid::parse_str(ORG).unwrap()
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn eur() -> CurrencyCode {
    CurrencyCode::parse("EUR").unwrap()
}

/// A fixed instant, whole seconds, so `expires_at` compares exactly after a round trip.
fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 19, 9, 0, 0).unwrap()
}

fn secs(n: i64) -> Duration {
    Duration::seconds(n)
}

fn is_code(err: &AppError, code: &str) -> bool {
    matches!(err, AppError::Domain(m) if m.starts_with(code))
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

struct Db {
    pool: PgPool,
    admin: PgPool,
    schema: String,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. Point it at a database ending in `_test`.")
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("b05_sr1");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to test db");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create schema");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to schema");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        Db {
            pool,
            admin,
            schema,
        }
    }

    async fn close(self) {
        self.pool.close().await;
        let _ = sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.schema
        )))
        .execute(&self.admin)
        .await;
        self.admin.close().await;
    }

    async fn branch(&self) -> BranchId {
        let id = unique("br");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert branch");
        BranchId::new(id)
    }

    /// The branch's `inventory_positions` row of `currency` at `quantity` (cost 35 THB a unit).
    async fn position(&self, branch: &BranchId, currency: &str, quantity: &str) {
        sqlx::query(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
             VALUES ($1::uuid, $2, $3, $4::numeric, $4::numeric * 35) \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE SET quantity = EXCLUDED.quantity",
        )
        .bind(ORG)
        .bind(branch.as_str())
        .bind(currency)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .expect("insert position");
    }

    /// One ACTIVE quote at `branch` expiring at `expires_at`, with one line `side currency quantity`.
    /// Answers `(quote_id, quote_line_id)`.
    async fn quote(
        &self,
        branch: &BranchId,
        side: &str,
        currency: &str,
        quantity: &str,
        expires_at: DateTime<Utc>,
    ) -> (Uuid, Uuid) {
        let (quote_id, pub_id, req_id, rate_line_id, line_id) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let user: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("user"))
        .fetch_one(&self.pool)
        .await
        .expect("insert user");
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
             VALUES ($1, $2::uuid, 'RATE_PUBLICATION', $3, $4, 'test', '1', '{}'::jsonb, $5, now(), \
             'APPROVED', $6)",
        )
        .bind(req_id)
        .bind(ORG)
        .bind(branch.as_str())
        .bind(pub_id)
        .bind(&user)
        .bind(format!("k-{req_id}"))
        .execute(&self.pool)
        .await
        .expect("insert approval request");
        sqlx::query(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, \
             draft_id, published_by) VALUES ($1, $2, \
             (SELECT COALESCE(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $2), now(), $3, NULL, $4)",
        )
        .bind(pub_id)
        .bind(branch.as_str())
        .bind(req_id)
        .bind(&user)
        .execute(&self.pool)
        .await
        .expect("insert rate publication");
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, \
             unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES ($1, $2, $3, 'ALL', 1.0, 34.0, 35.0, '{}'::jsonb)",
        )
        .bind(rate_line_id)
        .bind(pub_id)
        .bind(currency)
        .execute(&self.pool)
        .await
        .expect("insert published rate line");
        sqlx::query(
            "INSERT INTO quotes (id, organization_id, branch_id, type, channel, status, publication_id, \
             expires_at, created_by, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, 'INDICATIVE', 'DESK', 'ACTIVE', $4, $5, 'test', now(), now())",
        )
        .bind(quote_id)
        .bind(ORG)
        .bind(branch.as_str())
        .bind(pub_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .expect("insert quote");
        sqlx::query(
            "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, shop_side, \
             currency, denomination_group, quantity, unit_size, base_rate, final_rate, thb_amount, \
             fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, 'ALL', $7::numeric, 1.0, 35.0, 35.0, 0, 0, 0, 1, now(), now())",
        )
        .bind(line_id)
        .bind(ORG)
        .bind(quote_id)
        .bind(rate_line_id)
        .bind(side)
        .bind(currency)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .expect("insert quote line");
        (quote_id, line_id)
    }

    async fn reserve(&self, line: Uuid, now: DateTime<Utc>) -> AppResult<ReservationId> {
        let mut tx = self.pool.begin().await.unwrap();
        let got = PgStockReservations::new()
            .reserve_in(
                &mut tx,
                &ReserveLine {
                    quote_line_id: line,
                    denomination_id: None,
                },
                now,
            )
            .await;
        if got.is_ok() {
            tx.commit().await.unwrap();
        }
        got
    }

    async fn consume(&self, quote: Uuid, transaction: Uuid, now: DateTime<Utc>) -> AppResult<()> {
        let mut tx = self.pool.begin().await.unwrap();
        let got = PgStockReservations::new()
            .consume_for_quote_in(&mut tx, quote, transaction, now)
            .await;
        if got.is_ok() {
            tx.commit().await.unwrap();
        }
        got
    }

    async fn release(&self, quote: Uuid, reason: ReleaseReason) -> AppResult<()> {
        let mut tx = self.pool.begin().await.unwrap();
        let got = PgStockReservations::new()
            .release_for_quote_in(&mut tx, quote, reason)
            .await;
        if got.is_ok() {
            tx.commit().await.unwrap();
        }
        got
    }

    async fn availability(
        &self,
        branch: &BranchId,
        currency: CurrencyCode,
        now: DateTime<Utc>,
    ) -> StockAvailability {
        let mut conn = self.pool.acquire().await.unwrap();
        PgStockReservations::new()
            .availability_in(&mut conn, org(), branch, currency, now)
            .await
            .unwrap_or_else(|e| panic!("availability_in: {e}"))
    }

    /// `status|released_reason|transaction_id|quantity|expires_at` of every row of `line`, oldest first.
    async fn rows(&self, line: Uuid) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT status || '|' || COALESCE(released_reason, '-') || '|' || \
                    COALESCE(transaction_id::text, '-') || '|' || quantity::text || '|' || \
                    to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS') \
               FROM stock_reservations WHERE quote_line_id = $1 ORDER BY created_at, id",
        )
        .bind(line)
        .fetch_all(&self.pool)
        .await
        .expect("read stock_reservations")
    }

    async fn count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM stock_reservations")
            .fetch_one(&self.pool)
            .await
            .expect("count stock_reservations")
    }
}

fn reservation(status: ReservationStatus, expires_at: DateTime<Utc>) -> StockReservation {
    StockReservation {
        id: ReservationId(Uuid::new_v4()),
        organization_id: org(),
        branch_id: BranchId::new("MAIN"),
        quote_line_id: Uuid::new_v4(),
        currency: usd(),
        denomination_id: None,
        quantity: ForeignAmount::from_decimal(d("60")),
        expires_at,
        status,
        transaction_id: None,
    }
}

// ------------------------------------------------------------ pure (no database)

#[test]
fn d1_available_is_on_hand_minus_reserved() {
    let a = StockAvailability {
        on_hand: d("100"),
        reserved: d("60"),
    };
    assert_eq!(a.available(), d("40"));
    let short = StockAvailability {
        on_hand: d("-5"),
        reserved: d("0"),
    };
    assert_eq!(short.available(), d("-5"));
}

#[test]
fn d3_active_is_reserved_and_before_expiry() {
    let exp = t0() + secs(300);
    assert!(reservation(ReservationStatus::Reserved, exp).is_active(t0()));
    assert!(!reservation(ReservationStatus::Reserved, exp).is_active(exp));
    assert!(!reservation(ReservationStatus::Reserved, exp).is_active(exp + secs(1)));
    assert!(!reservation(ReservationStatus::Consumed, exp).is_active(t0()));
    assert!(
        !reservation(ReservationStatus::Released(ReleaseReason::Cancelled), exp).is_active(t0())
    );
}

#[test]
fn stored_texts_are_the_tables() {
    assert_eq!(ReservationStatus::Reserved.as_str(), "RESERVED");
    assert_eq!(ReservationStatus::Consumed.as_str(), "CONSUMED");
    assert_eq!(
        ReservationStatus::Released(ReleaseReason::Expired).as_str(),
        "RELEASED"
    );
    assert_eq!(ReleaseReason::Expired.as_str(), "EXPIRED");
    assert_eq!(ReleaseReason::Cancelled.as_str(), "CANCELLED");
    assert_eq!(
        ReleaseReason::parse("EXPIRED"),
        Some(ReleaseReason::Expired)
    );
    assert_eq!(
        ReleaseReason::parse("CANCELLED"),
        Some(ReleaseReason::Cancelled)
    );
    assert_eq!(ReleaseReason::parse("cancelled"), None);
}

// ------------------------------------------------------------ the store

#[tokio::test]
async fn s1_no_reservation_available_is_on_hand() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;

    let a = db.availability(&b, usd(), t0()).await;
    assert_eq!((a.on_hand, a.reserved), (d("100"), d("0")));
    assert_eq!(a.available(), d("100"));

    // no position row at all: everything is zero
    let none = db.availability(&b, eur(), t0()).await;
    assert_eq!((none.on_hand, none.reserved), (d("0"), d("0")));
    db.close().await;
}

#[tokio::test]
async fn s2_reserve_holds_with_the_quotes_expiry() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let exp = t0() + secs(300);
    let (_, line) = db.quote(&b, "SELL", "USD", "60", exp).await;

    db.reserve(line, t0()).await.expect("reserve 60 of 100");

    assert_eq!(
        db.rows(line).await,
        ["RESERVED|-|-|60.0000|2026-10-19T09:05:00"]
    );
    let a = db.availability(&b, usd(), t0()).await;
    assert_eq!(
        (a.on_hand, a.reserved, a.available()),
        (d("100"), d("60"), d("40"))
    );
    let mut conn = db.pool.acquire().await.unwrap();
    assert_eq!(
        active_reserved_in(&mut conn, org(), &b, usd(), t0())
            .await
            .expect("active_reserved_in"),
        d("60")
    );
    drop(conn);
    db.close().await;
}

#[tokio::test]
async fn s3_reserve_beyond_available_is_refused_and_writes_nothing() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let exp = t0() + secs(300);
    let (_, first) = db.quote(&b, "SELL", "USD", "60", exp).await;
    let (_, second) = db.quote(&b, "SELL", "USD", "50", exp).await;

    db.reserve(first, t0()).await.expect("reserve 60");
    let err = db
        .reserve(second, t0())
        .await
        .expect_err("50 > available 40");
    assert!(is_code(&err, "stockInsufficient"), "{err:?}");
    assert_eq!(db.rows(second).await, Vec::<String>::new());
    assert_eq!(db.count().await, 1);

    // no position row: nothing is available
    let (_, third) = db.quote(&b, "SELL", "EUR", "1", exp).await;
    let err = db.reserve(third, t0()).await.expect_err("no EUR at all");
    assert!(is_code(&err, "stockInsufficient"), "{err:?}");
    db.close().await;
}

#[tokio::test]
async fn s4_two_concurrent_reserves_of_the_last_units_one_wins() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let exp = t0() + secs(300);
    let (_, a) = db.quote(&b, "SELL", "USD", "60", exp).await;
    let (_, c) = db.quote(&b, "SELL", "USD", "60", exp).await;

    let (ra, rc) = tokio::join!(db.reserve(a, t0()), db.reserve(c, t0()));
    let oks = [ra.is_ok(), rc.is_ok()].iter().filter(|x| **x).count();
    assert_eq!(oks, 1, "exactly one wins: {ra:?} {rc:?}");
    let loser = if ra.is_err() { ra } else { rc };
    assert!(is_code(&loser.unwrap_err(), "stockInsufficient"));
    assert_eq!(db.count().await, 1);
    assert_eq!(db.availability(&b, usd(), t0()).await.reserved, d("60"));
    db.close().await;
}

#[tokio::test]
async fn s5_an_expired_reservation_holds_nothing() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let exp = t0() + secs(300);
    let (_, line) = db.quote(&b, "SELL", "USD", "60", exp).await;
    db.reserve(line, t0()).await.expect("reserve");

    let at_expiry = db.availability(&b, usd(), exp).await;
    assert_eq!(
        (at_expiry.reserved, at_expiry.available()),
        (d("0"), d("100"))
    );
    // nothing is written by a read: the row is still RESERVED (no sweeper, D-3)
    assert_eq!(
        db.rows(line).await,
        ["RESERVED|-|-|60.0000|2026-10-19T09:05:00"]
    );
    db.close().await;
}

#[tokio::test]
async fn s6_the_next_reserve_releases_expired_rows_lazily() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let (_, old) = db.quote(&b, "SELL", "USD", "60", t0() + secs(300)).await;
    db.reserve(old, t0()).await.expect("reserve the old line");

    let later = t0() + secs(301);
    let (_, new) = db.quote(&b, "SELL", "USD", "80", later + secs(300)).await;
    db.reserve(new, later)
        .await
        .expect("80 fits once the old hold expired");

    assert_eq!(
        db.rows(old).await,
        ["RELEASED|EXPIRED|-|60.0000|2026-10-19T09:05:00"]
    );
    assert_eq!(
        db.rows(new).await,
        ["RESERVED|-|-|80.0000|2026-10-19T09:10:01"]
    );
    assert_eq!(db.availability(&b, usd(), later).await.reserved, d("80"));
    db.close().await;
}

#[tokio::test]
async fn s7_cancel_releases_at_once() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let (quote, line) = db.quote(&b, "SELL", "USD", "60", t0() + secs(300)).await;
    db.reserve(line, t0()).await.expect("reserve");

    db.release(quote, ReleaseReason::Cancelled)
        .await
        .expect("release");

    assert_eq!(
        db.rows(line).await,
        ["RELEASED|CANCELLED|-|60.0000|2026-10-19T09:05:00"]
    );
    let a = db.availability(&b, usd(), t0() + secs(1)).await;
    assert_eq!((a.reserved, a.available()), (d("0"), d("100")));

    // a quote with nothing reserved: Ok, nothing written
    let (bare, _) = db.quote(&b, "SELL", "USD", "1", t0() + secs(300)).await;
    db.release(bare, ReleaseReason::Cancelled)
        .await
        .expect("nothing to release");
    assert_eq!(db.count().await, 1);
    db.close().await;
}

#[tokio::test]
async fn s8_consume_marks_the_quotes_rows_consumed_with_the_transaction() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let (quote, line) = db.quote(&b, "SELL", "USD", "60", t0() + secs(300)).await;
    db.reserve(line, t0()).await.expect("reserve");
    let txn = Uuid::new_v4();

    db.consume(quote, txn, t0() + secs(10))
        .await
        .expect("consume");

    assert_eq!(
        db.rows(line).await,
        [format!("CONSUMED|-|{txn}|60.0000|2026-10-19T09:05:00")]
    );
    assert_eq!(
        db.availability(&b, usd(), t0() + secs(10)).await.reserved,
        d("0")
    );
    db.close().await;
}

#[tokio::test]
async fn s9_consume_after_expiry_re_reserves_when_stock_allows() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let (quote, line) = db.quote(&b, "SELL", "USD", "60", t0() + secs(300)).await;
    db.reserve(line, t0()).await.expect("reserve");
    let txn = Uuid::new_v4();

    db.consume(quote, txn, t0() + secs(301))
        .await
        .expect("60 still available after expiry");

    let rows = db.rows(line).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        rows[0].starts_with(&format!("CONSUMED|-|{txn}|60.0000|")),
        "{rows:?}"
    );
    assert_eq!(db.count().await, 1);
    db.close().await;
}

#[tokio::test]
async fn s10_consume_after_expiry_refuses_when_stock_is_gone() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let (quote, line) = db.quote(&b, "SELL", "USD", "60", t0() + secs(300)).await;
    db.reserve(line, t0()).await.expect("reserve");
    // the stock fell to 30 while the hold had expired
    db.position(&b, "USD", "30").await;

    let err = db
        .consume(quote, Uuid::new_v4(), t0() + secs(301))
        .await
        .expect_err("60 > available 30");
    // #3819 (claude/a90ab16c's ruling): a reservation refusal answers its own code, never stockInsufficient
    assert!(is_code(&err, "stockReserved"), "{err:?}");
    // rolled back: the row is as it was
    assert_eq!(
        db.rows(line).await,
        ["RESERVED|-|-|60.0000|2026-10-19T09:05:00"]
    );
    db.close().await;
}

#[tokio::test]
async fn s11_the_same_line_twice_replays_the_live_row() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let (_, line) = db.quote(&b, "SELL", "USD", "60", t0() + secs(300)).await;

    let first = db.reserve(line, t0()).await.expect("first");
    let again = db.reserve(line, t0() + secs(5)).await.expect("replay");

    assert_eq!(first, again);
    assert_eq!(db.count().await, 1);
    assert_eq!(
        db.availability(&b, usd(), t0() + secs(5)).await.reserved,
        d("60")
    );
    db.close().await;
}

#[tokio::test]
async fn s12_consume_of_a_quote_with_no_live_row_is_a_no_op() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let (quote, line) = db.quote(&b, "SELL", "USD", "60", t0() + secs(300)).await;
    db.reserve(line, t0()).await.expect("reserve");
    let txn = Uuid::new_v4();
    db.consume(quote, txn, t0() + secs(1))
        .await
        .expect("consume");

    db.consume(quote, txn, t0() + secs(2))
        .await
        .expect("a second consume changes nothing");
    let (bare, _) = db.quote(&b, "SELL", "USD", "1", t0() + secs(300)).await;
    db.consume(bare, Uuid::new_v4(), t0() + secs(2))
        .await
        .expect("no reservation, nothing to consume");

    assert_eq!(
        db.rows(line).await,
        [format!("CONSUMED|-|{txn}|60.0000|2026-10-19T09:05:00")]
    );
    assert_eq!(db.count().await, 1);
    db.close().await;
}

#[tokio::test]
async fn s15_other_branches_and_currencies_are_unaffected() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    let other = db.branch().await;
    db.position(&b, "USD", "100").await;
    db.position(&b, "EUR", "100").await;
    db.position(&other, "USD", "100").await;
    let (_, line) = db.quote(&b, "SELL", "USD", "60", t0() + secs(300)).await;
    db.reserve(line, t0()).await.expect("reserve");

    assert_eq!(db.availability(&other, usd(), t0()).await.reserved, d("0"));
    assert_eq!(db.availability(&b, eur(), t0()).await.reserved, d("0"));
    assert_eq!(db.availability(&b, usd(), t0()).await.reserved, d("60"));
    db.close().await;
}

#[tokio::test]
async fn s20_buy_lines_and_thb_lines_are_not_reserved() {
    let db = Db::fresh().await;
    let b = db.branch().await;
    db.position(&b, "USD", "100").await;
    let (_, buy) = db.quote(&b, "BUY", "USD", "10", t0() + secs(300)).await;
    let (_, thb) = db.quote(&b, "SELL", "THB", "10", t0() + secs(300)).await;

    for line in [buy, thb] {
        let err = db.reserve(line, t0()).await.expect_err("not reservable");
        assert!(is_code(&err, "validationFailed"), "{err:?}");
    }
    assert_eq!(db.count().await, 0);
    db.close().await;
}

#[tokio::test]
async fn an_unknown_quote_line_is_refused() {
    let db = Db::fresh().await;
    let err = db
        .reserve(Uuid::new_v4(), t0())
        .await
        .expect_err("no such line");
    assert!(
        is_code(&err, "validationFailed") || is_code(&err, "staleVersion"),
        "{err:?}"
    );
    assert_eq!(db.count().await, 0);
    db.close().await;
}
