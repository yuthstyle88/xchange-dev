//! #4995 (Q3 of #4961, three-decimal currencies): custody JSON stores amounts as decimal strings, and a forward
//! migration rewrites the rows written in the old encoding (integer hundredths). Design
//! `docs/design/three-decimal-currencies.md` §3, §4 row K6, §5.
//!
//! Written before the implementation by claude/b873aa20 for the lane controller claude/7ca80c. Do not edit, delete or
//! weaken these tests; add your own in another file.
//!
//! Rows (#4995 §3):
//! - K6. Rows written in the old encoding, then migrated, read back with the same values (`1050` → `"10.50"`), in
//!   all three homes: `tx_drafts.data->'amount'`, `cash_exceptions.data` (`targetDifference`, `cashVariance`,
//!   `safeVariance`) and `ops_state.target->'amounts'`. A figure stored as text (beyond 64 bits) is converted the same
//!   way; a row without the keys is untouched.
//! - C1. A new draft of `KWD 1.125` is stored as the decimal string `"1.125"` and reads back as `1.125`.
//! - C2. An integer left in a row after the migration is refused (`undecodable`), not read at the wrong scale.
//!
//! How each row runs:
//! - K6 applies every migration before the custody-amounts migration (found by its description, so a renumbered
//!   slot still works), seeds the rows the way `custody_repository.rs` writes them today, then applies the rest.
//!   Before the migration exists, the second step applies nothing: the rows read back as before and the test fails on
//!   the stored encoding.
//! - C1 writes its new draft through `PgCustodyRepository::store_state_in`, cloned from a draft seeded as
//!   `"1.125"`, so it needs neither the desk form (Q4) nor `draftBegin` (Q6).
//! - The reads go through `PgCustodyRepository::load_state` and compare each amount's `Display` text as a decimal, so
//!   the file compiles on today's `Money` / `MoneyDelta` and on Q2's `ForeignAmount` / `QuantityDelta`.
//!
//! Needs `TEST_DATABASE_URL` naming a database ending in `_test`; nothing skips. Each test works in a schema of its
//! own.

use std::fmt::Display;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;

const MUST_FINISH: Duration = Duration::from_secs(30);
const B01: &str = "XC-B01";
/// The migration's file is `<slot>_custody_amounts_decimal.sql`; sqlx describes it with spaces.
const MIGRATION: &str = "custody amounts decimal";
const AT: i64 = 1_791_262_800_000;

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
    migrator: Migrator,
}

impl TestDb {
    /// A fresh schema with no migration applied yet.
    async fn empty() -> Self {
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
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        let schema = xc_testdb::unique_schema_name("custdec");
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
        let migrator = Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations");
        Self {
            pool,
            schema,
            migrator,
        }
    }

    /// The custody-amounts migration's version, if it exists yet.
    fn slot(&self) -> Option<i64> {
        self.migrator
            .migrations
            .iter()
            .find(|m| m.description.replace('_', " ") == MIGRATION)
            .map(|m| m.version)
    }

    /// Every migration before the custody-amounts one (all of them while it does not exist).
    async fn migrate_before_slot(&self) {
        let to = self.slot().map_or(i64::MAX, |v| v - 1);
        self.migrator
            .run_to(to, &self.pool)
            .await
            .expect("apply the migrations before the custody-amounts migration");
        self.branch().await;
    }

    /// The rest of the migrations, the custody-amounts one included.
    async fn migrate_rest(&self) {
        self.migrator
            .run(&self.pool)
            .await
            .expect("apply the rest of api/migrations");
    }

    /// A schema with every migration applied and the branch row.
    async fn migrated() -> Self {
        let db = Self::empty().await;
        db.migrator
            .run(&db.pool)
            .await
            .expect("apply api/migrations");
        db.branch().await;
        db
    }

    async fn branch(&self) {
        self.exec(&format!(
            "INSERT INTO branches (id, name) VALUES ('{B01}', 'B01')"
        ))
        .await;
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn json(&self, sql: &str) -> Value {
        sqlx::query_scalar::<_, Value>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    fn repo(&self) -> PgCustodyRepository {
        PgCustodyRepository::new(self.pool.clone())
    }

    async fn state(&self) -> CustodyState {
        self.repo()
            .load_state(&BranchId::new(B01))
            .await
            .unwrap_or_else(|e| panic!("custody state of {B01} does not load: {e:?}"))
    }

    async fn finish(self) {
        let TestDb { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    // ------------------------------------------------------------ seeds, in the stored shapes

    async fn ops_state(&self, target: Option<Value>) {
        let target = match target {
            Some(amounts) => format!(
                "'{}'::jsonb",
                json!({ "amounts": amounts, "by": "FIN1", "at": AT })
            ),
            None => "NULL".to_owned(),
        };
        self.exec(&format!(
            "INSERT INTO ops_state (branch_id, phase, target) VALUES ('{B01}', 'OPEN', {target})"
        ))
        .await;
    }

    async fn draft(&self, id: &str, position: i32, currency: &str, amount: Value) {
        let data = json!({
            "shiftId": "SH-1",
            "employeeId": "F1",
            "side": "BUY",
            "currency": currency,
            "amount": amount,
            "at": AT,
            "status": "OPEN",
            "documentRef": null,
        });
        self.exec(&format!(
            "INSERT INTO tx_drafts (id, branch_id, position, data) \
             VALUES ('{id}', '{B01}', {position}, '{data}'::jsonb)"
        ))
        .await;
    }

    async fn exception(&self, id: &str, position: i32, amounts: [Value; 3]) -> Value {
        let [target_difference, cash_variance, safe_variance] = amounts;
        let data = json!({
            "requestId": format!("count-{id}"),
            "status": "OPEN",
            "targetDifference": target_difference,
            "cashVariance": cash_variance,
            "safeVariance": safe_variance,
            "note": "short at close",
            "at": AT,
            "reviewedBy": null,
            "reviewedAt": null,
        });
        self.exec(&format!(
            "INSERT INTO cash_exceptions (id, branch_id, position, data) \
             VALUES ('{id}', '{B01}', {position}, '{data}'::jsonb)"
        ))
        .await;
        data
    }
}

/// An amount's text as a decimal, so `10.50`, `10.5` and `10.5000` compare equal.
fn dec<T: Display + ?Sized>(amount: &T) -> Decimal {
    let text = amount.to_string();
    Decimal::from_str(&text)
        .unwrap_or_else(|e| panic!("{text:?} is not a decimal: {e}"))
        .normalize()
}

fn d(text: &str) -> Decimal {
    dec(text)
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a currency code")
}

/// A stored amount is a JSON string holding `want` as a decimal (any scale).
fn a_decimal_string(stored: &Value, want: &str, what: &str) {
    match stored {
        Value::String(text) => assert_eq!(
            dec(text),
            d(want),
            "{what}: stored {stored}, want the decimal string of {want}"
        ),
        other => panic!("{what}: stored {other}, want a JSON string holding {want}"),
    }
}

// ------------------------------------------------------------ K6

/// K6: drafts, the target and a cash exception written in the old encoding read back with the same values after the
/// migration, and are then stored as decimal strings with two places. A row without the keys is untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k6_rows_in_the_old_encoding_read_back_the_same_after_the_migration() {
    let db = TestDb::empty().await;
    db.migrate_before_slot().await;
    db.ops_state(Some(json!({ "THB": 10_000_000, "USD": 1050 })))
        .await;
    db.draft("draft-1", 0, "USD", json!(1050)).await;
    db.exception(
        "exc-1",
        0,
        [json!({ "USD": 1050 }), json!({ "USD": -50 }), Value::Null],
    )
    .await;
    let untouched = db
        .exception("exc-2", 1, [Value::Null, Value::Null, Value::Null])
        .await;

    db.migrate_rest().await;

    // The values read back as they were written.
    let state = db.state().await;
    assert_eq!(state.drafts.len(), 1, "{state:?}");
    assert_eq!(dec(&state.drafts[0].amount), d("10.50"), "draft-1 amount");
    let target = state.target.as_ref().expect("the target");
    assert_eq!(
        dec(&target.amounts.get(code("USD"))),
        d("10.50"),
        "target USD"
    );
    assert_eq!(
        dec(&target.amounts.get(code("THB"))),
        d("100000"),
        "target THB"
    );
    assert_eq!(state.exceptions.len(), 2, "{state:?}");
    let exc = &state.exceptions[0];
    let difference = exc
        .target_difference
        .as_ref()
        .expect("exc-1 targetDifference");
    assert_eq!(
        dec(&difference.get(code("USD"))),
        d("10.50"),
        "exc-1 targetDifference USD"
    );
    let variance = exc.cash_variance.as_ref().expect("exc-1 cashVariance");
    assert_eq!(
        dec(&variance.get(code("USD"))),
        d("-0.50"),
        "exc-1 cashVariance USD"
    );
    assert!(exc.safe_variance.is_none(), "{exc:?}");

    // The migration stored them as decimal strings with two places.
    let amount = db
        .json(&format!(
            "SELECT data->'amount' FROM tx_drafts WHERE id = 'draft-1' AND branch_id = '{B01}'"
        ))
        .await;
    assert_eq!(amount, json!("10.50"), "tx_drafts draft-1 amount");
    let amounts = db
        .json(&format!(
            "SELECT target->'amounts' FROM ops_state WHERE branch_id = '{B01}'"
        ))
        .await;
    assert_eq!(
        amounts,
        json!({ "THB": "100000.00", "USD": "10.50" }),
        "ops_state target amounts"
    );
    let data = db
        .json("SELECT data FROM cash_exceptions WHERE id = 'exc-1'")
        .await;
    assert_eq!(
        data["targetDifference"],
        json!({ "USD": "10.50" }),
        "exc-1 {data}"
    );
    assert_eq!(
        data["cashVariance"],
        json!({ "USD": "-0.50" }),
        "exc-1 {data}"
    );
    assert_eq!(data["safeVariance"], Value::Null, "exc-1 {data}");
    let data = db
        .json("SELECT data FROM cash_exceptions WHERE id = 'exc-2'")
        .await;
    assert_eq!(data, untouched, "a row without amounts is untouched");

    db.finish().await;
}

/// K6: a figure the old encoding stored as text, because it does not fit 64 bits (`custody_repository.rs`
/// `minor_to_json`), is hundredths too and is converted the same way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn k6_a_figure_stored_as_text_beyond_64_bits_is_converted_the_same_way() {
    let db = TestDb::empty().await;
    db.migrate_before_slot().await;
    db.ops_state(None).await;
    db.exception(
        "exc-1",
        0,
        [
            Value::Null,
            json!({ "USD": "-92233720368547758070" }),
            json!({ "USD": 1 }),
        ],
    )
    .await;

    db.migrate_rest().await;

    let data = db
        .json("SELECT data FROM cash_exceptions WHERE id = 'exc-1'")
        .await;
    assert_eq!(
        data["cashVariance"],
        json!({ "USD": "-922337203685477580.70" }),
        "exc-1 {data}"
    );
    assert_eq!(
        data["safeVariance"],
        json!({ "USD": "0.01" }),
        "exc-1 {data}"
    );
    assert_eq!(data["targetDifference"], Value::Null, "exc-1 {data}");
    let target = db
        .json(&format!(
            "SELECT COALESCE(target, 'null'::jsonb) FROM ops_state WHERE branch_id = '{B01}'"
        ))
        .await;
    assert_eq!(target, Value::Null, "a branch with no target keeps none");

    db.finish().await;
}

// ------------------------------------------------------------ C1

/// C1: a new draft of `KWD 1.125` is written as the decimal string `"1.125"` and reads back as `1.125`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c1_a_new_draft_of_kwd_1_125_is_stored_as_a_decimal_string() {
    let db = TestDb::migrated().await;
    db.ops_state(None).await;
    db.draft("draft-1", 0, "KWD", json!("1.125")).await;

    let before = db.state().await;
    assert_eq!(before.drafts.len(), 1, "{before:?}");
    assert_eq!(
        dec(&before.drafts[0].amount),
        d("1.125"),
        "the seeded draft"
    );
    let mut after = before.clone();
    let mut draft = before.drafts[0].clone();
    draft.id = "draft-2".to_owned();
    after.drafts.push(draft);

    let repo = db.repo();
    let branch = BranchId::new(B01);
    let mut tx = db.pool.begin().await.expect("begin");
    repo.store_state_in(&mut tx, &branch, &before, &after)
        .await
        .unwrap_or_else(|e| panic!("store the new draft: {e:?}"));
    tx.commit().await.expect("commit");

    let stored = db
        .json(&format!(
            "SELECT data->'amount' FROM tx_drafts WHERE id = 'draft-2' AND branch_id = '{B01}'"
        ))
        .await;
    a_decimal_string(&stored, "1.125", "tx_drafts draft-2 amount");
    let state = db.state().await;
    let draft = state
        .drafts
        .iter()
        .find(|d| d.id == "draft-2")
        .expect("draft-2 reads back");
    assert_eq!(dec(&draft.amount), d("1.125"), "draft-2 amount");
    assert_eq!(draft.currency.as_str(), "KWD");

    db.finish().await;
}

/// C1, two places: a new `USD 10.50` draft is a decimal string too, never integer hundredths.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c1_a_new_draft_of_usd_10_50_is_stored_as_a_decimal_string() {
    let db = TestDb::migrated().await;
    db.ops_state(None).await;
    db.draft("draft-1", 0, "USD", json!("10.50")).await;

    let before = db.state().await;
    assert_eq!(
        dec(&before.drafts[0].amount),
        d("10.50"),
        "the seeded draft"
    );
    let mut after = before.clone();
    let mut draft = before.drafts[0].clone();
    draft.id = "draft-2".to_owned();
    after.drafts.push(draft);

    let repo = db.repo();
    let branch = BranchId::new(B01);
    let mut tx = db.pool.begin().await.expect("begin");
    repo.store_state_in(&mut tx, &branch, &before, &after)
        .await
        .unwrap_or_else(|e| panic!("store the new draft: {e:?}"));
    tx.commit().await.expect("commit");

    let stored = db
        .json(&format!(
            "SELECT data->'amount' FROM tx_drafts WHERE id = 'draft-2' AND branch_id = '{B01}'"
        ))
        .await;
    a_decimal_string(&stored, "10.50", "tx_drafts draft-2 amount");

    db.finish().await;
}

// ------------------------------------------------------------ C2

/// The custody state of the branch does not load, and the fault names the table.
async fn refused(db: &TestDb, table: &str) {
    match db.repo().load_state(&BranchId::new(B01)).await {
        Ok(state) => panic!("an integer amount in {table} was read as {state:?}"),
        Err(e) => {
            let text = format!("{e:?}");
            assert!(
                text.contains("undecodable") && text.contains(table),
                "want an undecodable-value fault naming {table}, got {text}"
            );
        }
    }
}

/// C2: an integer draft amount left after the migration is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_an_integer_draft_amount_after_the_migration_is_refused() {
    let db = TestDb::migrated().await;
    db.ops_state(None).await;
    db.draft("draft-1", 0, "USD", json!(1050)).await;
    refused(&db, "tx_drafts").await;
    db.finish().await;
}

/// C2: an integer in the target's amounts left after the migration is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_an_integer_target_amount_after_the_migration_is_refused() {
    let db = TestDb::migrated().await;
    db.ops_state(Some(json!({ "USD": 1050 }))).await;
    refused(&db, "ops_state").await;
    db.finish().await;
}

/// C2: an integer in a cash exception's variance left after the migration is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_an_integer_exception_variance_after_the_migration_is_refused() {
    let db = TestDb::migrated().await;
    db.ops_state(None).await;
    db.exception(
        "exc-1",
        0,
        [Value::Null, json!({ "USD": -50 }), Value::Null],
    )
    .await;
    refused(&db, "cash_exceptions").await;
    db.finish().await;
}
