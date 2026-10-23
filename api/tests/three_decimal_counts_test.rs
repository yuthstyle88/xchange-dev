//! #4997 (Q5 of #4961, three-decimal currencies): counts, transfers and the ledger readers keep 3-place balances, with
//! no skip and no rounding. Design `docs/design/three-decimal-currencies.md` §2 W7–W10, §3 (ledger reads), §4 rows
//! K4 and K7.
//!
//! Written before the implementation by claude/b873aa20 for the lane controller claude/7ca80c. Do not edit, delete or
//! weaken these tests; add your own in another file.
//!
//! Rows (#4997 §3):
//! - K4. An open count of `KWD 301 × 0.005 = 1.505` is accepted, and its total is stored as `1.505`
//!   (`count_service.rs` caps the scale at `min(minor_unit, 2)` today). As today: a `JPY 1 × 0.5` total is refused
//!   (`TooManyDecimals`).
//! - K7. A drawer holding `KWD 1.125`: `expected_in` lists `KWD 1.125` (W8 skips it today); `activity_in` and the
//!   shift balance read `1.125` (W9 and W10 round it to `1.13` today); a KWD transfer out of the drawer finds the
//!   stock (today the skipped balance reads 0 and the dispatch is `StockInsufficient`). As today: a drawer's
//!   `USD 300` reads `300` in all three readers.
//! - L1. A ledger sum with more places than the currency's `minor_unit` (`KWD 1.1255`, `JPY 0.5`) is refused by
//!   `expected_in`, `activity_in` and the shift balance with an internal error naming the currency, not skipped and
//!   not rounded.
//!
//! How each row runs:
//! - K7 and L1 seed the ledger directly: an OPENING posting `Bank → Drawer(shift)` through
//!   `LedgerCashPostings::post_in`, so they need neither the desk form (Q4) nor a desk save. The shift itself is opened
//!   by the count path (USD 300), as `crates/xc-cash/tests/moves_test.rs` does.
//! - K7's transfer moves `KWD 1 × 1` (face value 1), not `1.125`. A 3-place face value can't reach a transfer within
//!   #4997's File scope: `transfer_moves.rs:137` reads `face_value` with `Money::parse`, `cash_transfer.rs:131`
//!   `checked_line_minor` takes it as `Money`, and the kinds post `Decimal::new(minor, 2)` (`kinds/safe.rs:121`).
//! - The reads go through the xc-cash public API (`CashCountService`, `LedgerCashPostings`, `PgLedgerShiftBalances`,
//!   `TransferFlow`) and compare each amount's `Display` text as a decimal, so the file compiles on today's `Money` /
//!   `MoneyDelta` and on Q2's `ForeignAmount` / `QuantityDelta`.
//!
//! Needs `TEST_DATABASE_URL` naming a database ending in `_test`; nothing skips. Each test works in a schema of its
//! own.

use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::domain::transfer::cash_transfer::{NewTransfer, NewTransferLine, TransferKind};
use xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;
use xc_testdb::people::{RoleRow, seed_person};

const MUST_FINISH: Duration = Duration::from_secs(30);
const ORG: &str = "00000000-0000-0000-0000-000000000001";

static POSTING: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a currency code")
}

/// An amount's text as a decimal, so `1.125`, `1.1250` and `1.13` compare by value.
fn dec<T: Display + ?Sized>(amount: &T) -> Decimal {
    let text = amount.to_string();
    Decimal::from_str(&text)
        .unwrap_or_else(|e| panic!("{text:?} is not a decimal: {e}"))
        .normalize()
}

fn d(text: &str) -> Decimal {
    dec(text)
}

// ------------------------------------------------------------ database

struct Db {
    pool: PgPool,
    schema: String,
}

struct Shop {
    branch: BranchId,
    teller: Actor,
    manager: Actor,
    finance: Actor,
    exec: Actor,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("thrdec");
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
            .max_connections(4)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        Db { pool, schema }
    }

    async fn finish(self) {
        let Db { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    async fn shop(&self) -> Shop {
        let id = unique("b1");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert the branch");
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert ops_state");
        let branch = BranchId::new(id);
        Shop {
            teller: self.person("Ploy", &[(Role::Teller, Some(&branch))]).await,
            manager: self
                .person("Krit", &[(Role::BranchManager, Some(&branch))])
                .await,
            finance: self.person("Pim", &[(Role::Finance, Some(&branch))]).await,
            exec: self.person("Exec", &[(Role::Executive, None)]).await,
            branch,
        }
    }

    /// A user holding `roles`, each on a branch (`None` = organisation-wide), and their `Actor`.
    async fn person(&self, name: &str, roles: &[(Role, Option<&BranchId>)]) -> Actor {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let role_rows: Vec<RoleRow> = roles
            .iter()
            .map(|(role, branch)| RoleRow {
                role_code: role.as_str(),
                branch_id: branch.map(|b| b.as_str()),
                valid_from: from,
                valid_to: None,
            })
            .collect();
        let mut conn = self.pool.acquire().await.expect("acquire a connection");
        let id = seed_person(&mut conn, ORG, name, &role_rows)
            .await
            .expect("seed the person");
        Actor::new(
            &id,
            ORG,
            roles
                .iter()
                .map(|(r, b)| ScopedRole::new(*r, b.cloned(), from, None))
                .collect(),
            None,
        )
    }

    /// The denomination of `currency` with `face`, added for the test when the register has none.
    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric \
             ORDER BY id LIMIT 1",
        )
        .bind(currency)
        .bind(face)
        .fetch_optional(&self.pool)
        .await
        .expect("read denominations");
        if let Some(id) = found {
            return id;
        }
        sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ($1, $2::numeric, 'TEST') RETURNING id",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("add the {currency} {face} denomination: {e}"))
    }

    /// An OPENING posting of `amount` from outside into `to`, straight into the ledger.
    async fn fund(&self, to: CashPlace, currency: &str, amount: &str) {
        let mut tx = self.pool.begin().await.expect("begin");
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to,
                    currency: code(currency),
                    quantity: ForeignAmount::from_decimal(d(amount)),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: format!(
                            "thrdec-fund:{currency}:{}",
                            POSTING.fetch_add(1, Ordering::Relaxed)
                        ),
                        source_key: "fund".to_string(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("post {currency} {amount} into the ledger: {e:?}"));
        tx.commit().await.expect("commit the funding");
    }

    /// The teller's shift, opened by the count path with `USD 6 × 50` taken from the safe.
    async fn drawer(&self, s: &Shop) -> String {
        self.fund(CashPlace::Safe(s.branch.clone()), "USD", "300")
            .await;
        let counts = CashCountService::pg(self.pool.clone());
        let usd_50 = self.denomination("USD", "50").await;
        let count = counts
            .submit(
                &s.teller,
                SubmitCount {
                    branch: s.branch.clone(),
                    shift_id: None,
                    kind: CountKind::Open,
                    lines: vec![CountLine::new("USD", usd_50, 6)],
                    note: None,
                    idempotency_key: unique("open"),
                },
                at(0),
            )
            .await
            .expect("the teller counts the opening");
        counts
            .verify(&s.manager, &count.id, at(5))
            .await
            .expect("the manager verifies the opening")
            .shift_id
            .expect("the shift it opened")
    }

    async fn expected_in(&self, place: &CashPlace) -> AppResult<Vec<(CurrencyCode, Decimal)>> {
        let mut tx = self.pool.begin().await.expect("begin");
        let balances = LedgerCashPostings::new().expected_in(&mut tx, place).await;
        tx.rollback().await.expect("rollback");
        let balances = balances?;
        Ok(balances
            .non_zero()
            .into_iter()
            .map(|(c, amount)| (c, dec(&amount)))
            .collect())
    }

    async fn activity_in(&self, place: &CashPlace) -> AppResult<Vec<(CurrencyCode, Decimal)>> {
        let mut tx = self.pool.begin().await.expect("begin");
        let activity = LedgerCashPostings::new().activity_in(&mut tx, place).await;
        tx.rollback().await.expect("rollback");
        Ok(activity?
            .unwrap_or_default()
            .into_iter()
            .map(|(c, amount)| (c, dec(&amount)))
            .collect())
    }

    /// `(currency, expected)` of each shift-balance row.
    async fn shift_balance(&self, shift: &str) -> AppResult<Vec<(CurrencyCode, Decimal)>> {
        let rows = PgLedgerShiftBalances::new(self.pool.clone())
            .shift_balances(&shift.to_owned())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.currency, dec(&row.expected)))
            .collect())
    }
}

/// The amount of `currency` in a reader's rows, `None` when the currency is absent.
fn amount_of(rows: &[(CurrencyCode, Decimal)], currency: &str) -> Option<Decimal> {
    rows.iter()
        .find(|(c, _)| c.as_str() == currency)
        .map(|(_, amount)| *amount)
}

/// The read is refused with an internal error that names `currency`: not a domain refusal, not a value.
fn refused<T: std::fmt::Debug>(read: AppResult<T>, currency: &str, what: &str) {
    match read {
        Err(AppError::Internal(text)) => assert!(
            text.contains(currency),
            "{what}: the internal error should name {currency}, got {text:?}"
        ),
        Err(other) => panic!("{what}: want an internal error naming {currency}, got {other:?}"),
        Ok(value) => panic!("{what}: a sum beyond {currency}'s minor unit was read as {value:?}"),
    }
}

// ------------------------------------------------------------ K4

/// K4: an open count of `KWD 301 × 0.005 = 1.505` is accepted and its total is stored as `1.505`. Flips
/// `count_service.rs` `kwd_total_with_three_decimal_places_is_refused_at_submit_and_nothing_is_written`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k4_an_open_count_of_kwd_301_x_0_005_is_accepted() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let kwd = db.denomination("KWD", "0.005").await;

    let count = CashCountService::pg(db.pool.clone())
        .submit(
            &s.teller,
            SubmitCount {
                branch: s.branch.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: vec![CountLine::new("KWD", kwd, 301)],
                note: None,
                idempotency_key: unique("open_kwd"),
            },
            at(0),
        )
        .await
        .unwrap_or_else(|e| panic!("KWD 301 × 0.005 = 1.505 should be accepted, got {e:?}"));

    let counted: Decimal = sqlx::query_scalar(
        "SELECT counted FROM cash_count_totals WHERE count_id = $1::uuid AND currency = 'KWD'",
    )
    .bind(&count.id)
    .fetch_one(&db.pool)
    .await
    .expect("the KWD total of the count");
    assert_eq!(counted.normalize(), d("1.505"), "the stored KWD total");

    db.finish().await;
}

/// K4, as today: a total with more places than the currency's minor unit (`JPY 1 × 0.5`) is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k4_as_today_a_jpy_total_of_0_5_is_refused() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let jpy = db.denomination("JPY", "0.5").await;

    let res = CashCountService::pg(db.pool.clone())
        .submit(
            &s.teller,
            SubmitCount {
                branch: s.branch.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: vec![CountLine::new("JPY", jpy, 1)],
                note: None,
                idempotency_key: unique("open_jpy"),
            },
            at(0),
        )
        .await;
    match res {
        Err(AppError::Domain(text)) => assert_eq!(text, CashError::TooManyDecimals.to_string()),
        other => panic!("JPY 0.5 should be refused with TooManyDecimals, got {other:?}"),
    }

    db.finish().await;
}

// ------------------------------------------------------------ K7

/// K7 (W8): `expected_in` of a drawer holding `KWD 1.125` lists `KWD 1.125`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k7_expected_in_lists_kwd_1_125() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let shift = db.drawer(&s).await;
    let drawer = CashPlace::Drawer(shift.clone());
    db.fund(drawer.clone(), "KWD", "1.125").await;

    let rows = db
        .expected_in(&drawer)
        .await
        .unwrap_or_else(|e| panic!("expected_in of the drawer: {e:?}"));
    assert_eq!(amount_of(&rows, "KWD"), Some(d("1.125")), "{rows:?}");
    assert_eq!(amount_of(&rows, "USD"), Some(d("300")), "{rows:?}");

    db.finish().await;
}

/// K7 (W9): `activity_in` of the drawer reads `KWD 1.125`, not `1.13`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k7_activity_in_reads_kwd_1_125() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let shift = db.drawer(&s).await;
    let drawer = CashPlace::Drawer(shift.clone());
    db.fund(drawer.clone(), "KWD", "1.125").await;

    let rows = db
        .activity_in(&drawer)
        .await
        .unwrap_or_else(|e| panic!("activity_in of the drawer: {e:?}"));
    assert_eq!(amount_of(&rows, "KWD"), Some(d("1.125")), "{rows:?}");

    db.finish().await;
}

/// K7 (W10): the shift balance shows `KWD 1.125` expected, not `1.13`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k7_the_shift_balance_shows_kwd_1_125() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let shift = db.drawer(&s).await;
    db.fund(CashPlace::Drawer(shift.clone()), "KWD", "1.125")
        .await;

    let rows = db
        .shift_balance(&shift)
        .await
        .unwrap_or_else(|e| panic!("the shift balance: {e:?}"));
    assert_eq!(amount_of(&rows, "KWD"), Some(d("1.125")), "{rows:?}");

    db.finish().await;
}

/// K7 (W8 through the transfer's stock check, `transfer_flow.rs`): a KWD move out of a drawer holding `KWD 1.125`
/// finds the stock, and `KWD 0.125` stays in the drawer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k7_a_kwd_transfer_out_of_the_drawer_finds_the_stock() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let shift = db.drawer(&s).await;
    let drawer = CashPlace::Drawer(shift.clone());
    db.fund(drawer.clone(), "KWD", "1.125").await;
    let kwd_1 = db.denomination("KWD", "1").await;

    let flow = TransferFlow::pg(db.pool.clone());
    let t = flow
        .create(
            &s.finance,
            NewTransfer {
                kind: TransferKind::DrawerToSafe,
                from_branch: s.branch.clone(),
                to_branch: None,
                from_shift: Some(shift.clone()),
                to_shift: None,
                lines: vec![NewTransferLine {
                    currency: code("KWD"),
                    denomination_id: kwd_1,
                    qty: 1,
                }],
                note: None,
                idempotency_key: unique("move"),
            },
            at(10),
        )
        .await
        .expect("FINANCE creates the move");
    flow.submit(&s.finance, &t.id, at(11))
        .await
        .expect("submit");
    flow.decide(&s.exec, &t.id, Outcome::Approved, "ok", at(12))
        .await
        .expect("EXECUTIVE approves");
    flow.dispatch(&s.finance, &t.id, at(13))
        .await
        .unwrap_or_else(|e| panic!("KWD 1 out of a drawer holding KWD 1.125: {e:?}"));

    let rows = db
        .expected_in(&drawer)
        .await
        .unwrap_or_else(|e| panic!("expected_in of the drawer: {e:?}"));
    assert_eq!(amount_of(&rows, "KWD"), Some(d("0.125")), "{rows:?}");

    db.finish().await;
}

/// K7, as today: the drawer's `USD 300` reads `300` in all three readers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k7_as_today_usd_300_reads_300_in_every_reader() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let shift = db.drawer(&s).await;
    let drawer = CashPlace::Drawer(shift.clone());

    let expected = db.expected_in(&drawer).await.expect("expected_in");
    assert_eq!(amount_of(&expected, "USD"), Some(d("300")), "{expected:?}");
    let activity = db.activity_in(&drawer).await.expect("activity_in");
    assert_eq!(amount_of(&activity, "USD"), Some(d("300")), "{activity:?}");
    let balance = db.shift_balance(&shift).await.expect("shift balance");
    assert_eq!(amount_of(&balance, "USD"), Some(d("300")), "{balance:?}");

    db.finish().await;
}

// ------------------------------------------------------------ L1

/// L1: a drawer sum of `KWD 1.1255` (4 places, minor unit 3) is refused by every reader.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_a_kwd_sum_with_four_places_is_refused_not_skipped_or_rounded() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let shift = db.drawer(&s).await;
    let drawer = CashPlace::Drawer(shift.clone());
    db.fund(drawer.clone(), "KWD", "1.1255").await;

    refused(db.expected_in(&drawer).await, "KWD", "expected_in");
    refused(db.activity_in(&drawer).await, "KWD", "activity_in");
    refused(db.shift_balance(&shift).await, "KWD", "shift balance");

    db.finish().await;
}

/// L1: a drawer sum of `JPY 0.5` (minor unit 0) is refused by every reader.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_a_jpy_sum_of_0_5_is_refused_not_read_at_two_places() {
    let db = Db::fresh().await;
    let s = db.shop().await;
    let shift = db.drawer(&s).await;
    let drawer = CashPlace::Drawer(shift.clone());
    db.fund(drawer.clone(), "JPY", "0.5").await;

    refused(db.expected_in(&drawer).await, "JPY", "expected_in");
    refused(db.activity_in(&drawer).await, "JPY", "activity_in");
    refused(db.shift_balance(&shift).await, "JPY", "shift balance");

    db.finish().await;
}
