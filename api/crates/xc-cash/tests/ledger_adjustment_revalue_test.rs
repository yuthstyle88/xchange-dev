//! LA-8 up-front tests (design `docs/design/blueprint/ledger-inventory.md` §13.6, §13.9 rows LA-T25 to LA-T28):
//! `LedgerCashPostings::revalue_in`, a COST adjustment's cost-only change of a position.
//!
//! 🔴 Written by the sub-controller claude/f32b85db before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! The port takes a slice of `RevalueRequest`s sharing one source and writes one posting for all of them (the
//! skeleton's shape; see the sub-controller's note on PR #5278 about one source per adjustment). Every case here
//! sends one request.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_cash::application::reconcile::{Reconcile, ReconcileScope};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{
    CashPlace, CashPostings, PostingId, PostingRequest, RevalueRequest,
};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, MoneyDelta};

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B01: &str = "B01";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn source(kind: SourceType) -> SourceRef {
    SourceRef {
        source_type: kind,
        source_id: format!("ADJ-{}", unique("la8")),
        source_key: "post".to_owned(),
    }
}

fn leg(from: CashPlace, to: CashPlace, c: CurrencyCode, q: &str, s: &SourceRef) -> PostingRequest {
    PostingRequest {
        from,
        to,
        currency: c,
        quantity: ForeignAmount::from_decimal(d(q)),
        source: s.clone(),
        line: None,
        reverses: None,
    }
}

fn revalue(branch: &str, currency: CurrencyCode, minor: i128, s: &SourceRef) -> RevalueRequest {
    RevalueRequest {
        branch: BranchId::new(branch),
        currency,
        cost_delta_thb: MoneyDelta::from_minor(minor),
        source: s.clone(),
    }
}

async fn db() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip.")
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(
        options.get_database().unwrap_or("").ends_with("_test"),
        "TEST_DATABASE_URL must name a database ending in `_test`"
    );
    let schema = unique("la8");
    let admin = PgPoolOptions::new()
        .max_connections(1)
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
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    Migrator::new(dir.as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies");
    pool
}

async fn branch(pool: &PgPool, id: &str) {
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(id)
        .execute(pool)
        .await
        .expect("a branch");
}

async fn post(pool: &PgPool, reqs: Vec<PostingRequest>) {
    let mut tx = pool.begin().await.unwrap();
    LedgerCashPostings::new()
        .post_in(&mut tx, &reqs, None)
        .await
        .expect("the seed posts");
    tx.commit().await.unwrap();
}

/// `branch` buys `qty` USD for `thb` (one posting: Customer → SAFE USD, Bank → Customer THB).
async fn buy(pool: &PgPool, branch: &str, qty: &str, thb: &str) {
    let s = source(SourceType::Exchange);
    post(
        pool,
        vec![
            leg(
                CashPlace::Customer,
                CashPlace::Safe(BranchId::new(branch)),
                usd(),
                qty,
                &s,
            ),
            leg(
                CashPlace::Bank,
                CashPlace::Customer,
                CurrencyCode::THB,
                thb,
                &s,
            ),
        ],
    )
    .await;
}

/// `branch` sells `qty` USD for `thb`; answers the posting's source.
async fn sell(pool: &PgPool, branch: &str, qty: &str, thb: &str) -> SourceRef {
    let s = source(SourceType::Exchange);
    post(
        pool,
        vec![
            leg(
                CashPlace::Safe(BranchId::new(branch)),
                CashPlace::Customer,
                usd(),
                qty,
                &s,
            ),
            leg(
                CashPlace::Customer,
                CashPlace::Bank,
                CurrencyCode::THB,
                thb,
                &s,
            ),
        ],
    )
    .await;
    s
}

/// B01's USD position `900 / 29,250.00` (average 32.50).
async fn b01(pool: &PgPool) {
    branch(pool, B01).await;
    buy(pool, B01, "900", "29250.00").await;
    assert_eq!(position(pool, B01).await, Some((d("900"), d("29250.00"))));
}

/// `(quantity, cost_thb)` of `branch`'s USD position.
async fn position(pool: &PgPool, branch: &str) -> Option<(Decimal, Decimal)> {
    sqlx::query_as(
        "SELECT quantity, cost_thb FROM inventory_positions WHERE branch_id = $1 AND currency = 'USD'",
    )
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("read the position")
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// `(postings, entries, movements)` in the schema.
async fn written(pool: &PgPool) -> (i64, i64, i64) {
    (
        count(pool, "SELECT count(*) FROM ledger_postings").await,
        count(pool, "SELECT count(*) FROM ledger_entries").await,
        count(pool, "SELECT count(*) FROM inventory_movements").await,
    )
}

async fn run(pool: &PgPool, reqs: &[RevalueRequest]) -> Result<PostingId, String> {
    let mut tx = pool.begin().await.unwrap();
    let out = LedgerCashPostings::new()
        .revalue_in(&mut tx, reqs)
        .await
        .map_err(|e| format!("{e:?}"));
    if out.is_ok() {
        tx.commit().await.unwrap();
    }
    out
}

fn assert_validation(out: Result<PostingId, String>, what: &str) {
    let e = out.expect_err(&format!("{what} is refused"));
    assert!(
        e.contains("validationFailed"),
        "{what}: validationFailed, got {e}"
    );
}

/// `(movement_type, quantity_signed, cost_thb_signed, average_after, transaction_line_id is null)` of a posting's movements.
async fn movements(
    pool: &PgPool,
    s: &SourceRef,
) -> Vec<(String, Decimal, Decimal, Option<Decimal>, bool)> {
    sqlx::query_as(
        "SELECT m.movement_type, m.quantity_signed, m.cost_thb_signed, m.average_after, m.transaction_line_id IS NULL \
           FROM inventory_movements m JOIN ledger_postings p ON p.id = m.posting_id \
          WHERE p.source_id = $1 AND p.source_type = $2 ORDER BY m.id",
    )
    .bind(&s.source_id)
    .bind(s.source_type.as_str())
    .fetch_all(pool)
    .await
    .expect("read the movements")
}

// ── LA-T25 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn la_t25_a_revaluation_moves_the_cost_and_not_the_quantity() {
    let pool = db().await;
    b01(&pool).await;
    let s = source(SourceType::Adjustment);
    let id = run(&pool, &[revalue(B01, usd(), 45_000, &s)])
        .await
        .expect("USD +450.00 revalues");

    let (posting_source, entries): (String, i64) = sqlx::query_as(
        "SELECT p.source_type, (SELECT count(*) FROM ledger_entries e WHERE e.posting_id = p.id) \
           FROM ledger_postings p WHERE p.id = $1",
    )
    .bind(id.0)
    .fetch_one(&pool)
    .await
    .expect("the posting");
    assert_eq!(posting_source, "ADJUSTMENT");
    assert_eq!(entries, 0, "a cost-only posting has no entries (§13.6)");

    assert_eq!(
        movements(&pool, &s).await,
        vec![(
            "ADJUST".to_owned(),
            d("0"),
            d("450.00"),
            Some(d("33.00000000")),
            true
        )],
        "one ADJUST movement: quantity 0, +450.00, average 33, no line"
    );
    assert_eq!(position(&pool, B01).await, Some((d("900"), d("29700.00"))));

    let sold = sell(&pool, B01, "100", "3400.00").await;
    let dispose: Decimal = sqlx::query_scalar(
        "SELECT sum(m.cost_thb_signed) FROM inventory_movements m JOIN ledger_postings p ON p.id = m.posting_id \
          WHERE p.source_id = $1",
    )
    .bind(&sold.source_id)
    .fetch_one(&pool)
    .await
    .expect("the sale's movement");
    assert_eq!(
        dispose,
        d("-3300.00"),
        "the next SELL disposes at the new average"
    );
}

// ── LA-T26 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn la_t26_a_revaluation_that_cannot_hold_is_refused_and_writes_nothing() {
    let pool = db().await;
    b01(&pool).await;
    // B02's position is at 0, B03's below 0 (rows written directly: only the position is read).
    for (b, q) in [("B02", "0"), ("B03", "-5")] {
        branch(&pool, b).await;
        sqlx::query(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, version) \
             VALUES ($1::uuid, $2, 'USD', $3::numeric, 0, 1)",
        )
        .bind(ORG)
        .bind(b)
        .bind(q)
        .execute(&pool)
        .await
        .expect("a position");
    }
    let before = written(&pool).await;
    let adj = source(SourceType::Adjustment);
    assert_validation(
        run(&pool, &[revalue(B01, usd(), -3_000_000, &adj)]).await,
        "a delta taking the cost below zero (−30,000.00 on 29,250.00)",
    );
    assert_validation(
        run(&pool, &[revalue("B02", usd(), 10_000, &adj)]).await,
        "a position at 0",
    );
    assert_validation(
        run(&pool, &[revalue("B03", usd(), 10_000, &adj)]).await,
        "a negative position",
    );
    assert_validation(
        run(&pool, &[revalue(B01, CurrencyCode::THB, 10_000, &adj)]).await,
        "THB",
    );
    assert_validation(
        run(
            &pool,
            &[revalue(B01, usd(), 10_000, &source(SourceType::Exchange))],
        )
        .await,
        "a source that is not an adjustment",
    );
    assert_eq!(written(&pool).await, before, "nothing written");
    assert_eq!(position(&pool, B01).await, Some((d("900"), d("29250.00"))));
}

// ── LA-T27 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn la_t27_the_same_source_twice_writes_once() {
    let pool = db().await;
    b01(&pool).await;
    let s = source(SourceType::Adjustment);
    let first = run(&pool, &[revalue(B01, usd(), 45_000, &s)])
        .await
        .expect("the first revalues");
    let again = run(&pool, &[revalue(B01, usd(), 45_000, &s)])
        .await
        .expect("the repeat answers");
    assert_eq!(first, again, "the repeat answers the same posting");
    assert_eq!(movements(&pool, &s).await.len(), 1, "one movement");
    assert_eq!(
        count(
            &pool,
            &format!(
                "SELECT count(*) FROM ledger_postings WHERE source_id = '{}'",
                s.source_id
            )
        )
        .await,
        1,
        "one posting"
    );
    assert_eq!(position(&pool, B01).await, Some((d("900"), d("29700.00"))));
}

// ── LA-T28 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn la_t28_reconcile_stays_clean_after_a_revaluation() {
    let pool = db().await;
    b01(&pool).await;
    run(
        &pool,
        &[revalue(B01, usd(), 45_000, &source(SourceType::Adjustment))],
    )
    .await
    .expect("USD +450.00 revalues");
    let report = Reconcile::standard()
        .run(
            &pool,
            &ReconcileScope {
                organization_id: ORG.to_owned(),
                branch: Some(BranchId::new(B01)),
                period: None,
            },
        )
        .await
        .expect("the standard run");
    assert_eq!(report.checks.len(), 5, "{report:?}");
    for c in &report.checks {
        assert!(c.ok, "check {} stays clean: {:?}", c.id, c.offenders);
    }
}
