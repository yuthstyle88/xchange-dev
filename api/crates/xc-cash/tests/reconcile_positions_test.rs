//! Up-front tests for LG-7.2 (#1056; design `docs/design/blueprint/ledger-inventory.md` §6 check 2,
//! §3.2, §6.1): every `inventory_positions` row equals the sum of its movements, in quantity and in
//! cost. Written by the ticket author before the implementation. Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it
//! does not skip. Positions and movements are written directly (design §3.2's columns), as a
//! hand-edited row would be.

use xc_cash::application::reconcile::positions::{PositionsCheck, QUERY};

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_cash::application::reconcile::{Offender, Reconcile, ReconcileCheck, ReconcileScope};
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_ORG: &str = "00000000-0000-0000-0000-0000000000f2";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// A fresh schema with every migration applied; branches `B1` and `B2` of the organisation, and
/// `X1` of another organisation.
async fn db() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
             Point it at an existing database whose name ends in `_test`."
        )
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(
        options.get_database().unwrap_or("").ends_with("_test"),
        "TEST_DATABASE_URL must name a database ending in `_test`"
    );
    let schema = unique("lg7");
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
    Migrator::new(migrations_dir().as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies");
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1::uuid, 'OTHER', 'Asia/Bangkok', now())",
    )
    .bind(OTHER_ORG)
    .execute(&pool)
    .await
    .expect("another organisation");
    for (id, org) in [("B1", ORG), ("B2", ORG), ("X1", OTHER_ORG)] {
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(id)
            .bind(org)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("branch {id}: {e}"));
    }
    pool
}

#[allow(dead_code)]
fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn scope(branch: Option<&str>) -> ReconcileScope {
    ReconcileScope {
        organization_id: ORG.to_string(),
        branch: branch.map(BranchId::new),
        period: None,
    }
}

/// A cash account, inserted directly: answers its id as text.
#[allow(dead_code)]
async fn account(pool: &PgPool, org: &str, branch: &str, currency: &str, kind: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO cash_accounts (id, organization_id, branch_id, currency, kind, shift_id, \
             created_at) VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4, NULL, now()) \
         RETURNING id::text",
    )
    .bind(org)
    .bind(branch)
    .bind(currency)
    .bind(kind)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("account {branch} {currency} {kind}: {e}"))
}

/// A posting with `entries` (account id, currency, quantity, valuation), written with triggers off
/// (`session_replication_role = replica`) so a deliberately broken posting can be stored. Answers the
/// posting id as text.
async fn post_raw(
    pool: &PgPool,
    org: &str,
    branch: &str,
    source_type: &str,
    entries: &[(&str, &str, &str, &str)],
) -> String {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .expect("triggers off for this transaction");
    let id: String = sqlx::query_scalar(
        "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, \
             source_key, status, posted_at, posted_by) \
         VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4, 'k', 'POSTED', now(), 'test') \
         RETURNING id::text",
    )
    .bind(org)
    .bind(branch)
    .bind(source_type)
    .bind(unique("src"))
    .fetch_one(&mut *tx)
    .await
    .expect("a posting");
    for (acc, cur, q, v) in entries {
        sqlx::query(
            "INSERT INTO ledger_entries (posting_id, account_id, currency, quantity_signed, \
                 valuation_thb, created_at) \
             VALUES ($1::uuid, $2::uuid, $3, $4::numeric, $5::numeric, now())",
        )
        .bind(&id)
        .bind(acc)
        .bind(cur)
        .bind(q)
        .bind(v)
        .execute(&mut *tx)
        .await
        .expect("an entry");
    }
    tx.commit().await.expect("commit the posting");
    id
}

/// Runs `check` alone in a read-only transaction.
async fn offenders(
    pool: &PgPool,
    check: &dyn ReconcileCheck,
    scope: &ReconcileScope,
) -> Vec<Offender> {
    let mut tx = pool.begin().await.expect("begin");
    let out = check.run_in(&mut tx, scope).await.expect("the check runs");
    drop(tx);
    out
}

fn keys(offenders: &[Offender]) -> Vec<String> {
    offenders.iter().map(|o| o.key.clone()).collect()
}

/// `standard()` registers check `id`.
async fn assert_registered(pool: &PgPool, id: u8) {
    let report = Reconcile::standard()
        .run(pool, &scope(None))
        .await
        .expect("the standard run");
    assert!(
        report.checks.iter().any(|c| c.id == id),
        "check {id} is registered in Reconcile::standard()"
    );
}

/// 101 offenders give 100 plus the marker.
fn assert_capped(found: &[Offender]) {
    assert_eq!(found.len(), 101, "100 offenders plus the marker");
    assert_eq!(
        found[100].key, "truncated",
        "the marker is last: {:?}",
        found[100]
    );
    assert!(found[..100].iter().all(|o| o.key != "truncated"));
}

#[allow(dead_code)]
fn _uses(_: Arc<dyn ReconcileCheck>) {}

/// A posting to hang movements on (no entries; check 2 reads only the inventory tables).
async fn a_posting(pool: &PgPool, org: &str, branch: &str) -> String {
    post_raw(pool, org, branch, "ADJUSTMENT", &[]).await
}

async fn movement(
    pool: &PgPool,
    org: &str,
    branch: &str,
    currency: &str,
    kind: &str,
    q: &str,
    c: &str,
) {
    let posting = a_posting(pool, org, branch).await;
    sqlx::query(
        "INSERT INTO inventory_movements (organization_id, branch_id, currency, lot_id, posting_id, \
             transaction_line_id, movement_type, quantity_signed, cost_thb_signed, average_after, \
             created_at) \
         VALUES ($1::uuid, $2, $3, NULL, $4::uuid, NULL, $5, $6::numeric, $7::numeric, NULL, now())",
    )
    .bind(org)
    .bind(branch)
    .bind(currency)
    .bind(&posting)
    .bind(kind)
    .bind(q)
    .bind(c)
    .execute(pool)
    .await
    .expect("a movement");
}

async fn position(pool: &PgPool, org: &str, branch: &str, currency: &str, q: &str, c: &str) {
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, \
             version) VALUES ($1::uuid, $2, $3, $4::numeric, $5::numeric, 1)",
    )
    .bind(org)
    .bind(branch)
    .bind(currency)
    .bind(q)
    .bind(c)
    .execute(pool)
    .await
    .expect("a position");
}

/// B1 USD: ACQUIRE 100 for 3,200.00, then DISPOSE 40 at 1,280.00; the true position is 60 / 1,920.00.
async fn history(pool: &PgPool, org: &str, branch: &str) {
    movement(pool, org, branch, "USD", "ACQUIRE", "100", "3200.00").await;
    movement(pool, org, branch, "USD", "DISPOSE", "-40", "-1280.00").await;
}

#[tokio::test]
async fn a_position_that_equals_its_movements_is_clean() {
    let pool = db().await;
    history(&pool, ORG, "B1").await;
    position(&pool, ORG, "B1", "USD", "60", "1920.00").await;
    assert_eq!(
        offenders(&pool, &PositionsCheck, &scope(None)).await,
        vec![]
    );
    assert_eq!(PositionsCheck.id(), 2);
    pool.close().await;
}

#[tokio::test]
async fn a_quantity_edited_by_ten_is_reported_by_branch_and_currency() {
    let pool = db().await;
    history(&pool, ORG, "B1").await;
    position(&pool, ORG, "B1", "USD", "70", "1920.00").await;
    let found = offenders(&pool, &PositionsCheck, &scope(None)).await;
    assert_eq!(keys(&found), ["B1:USD"]);
    assert!(!found[0].detail.is_empty(), "the detail says what differs");
    pool.close().await;
}

#[tokio::test]
async fn a_cost_that_alone_differs_is_reported() {
    let pool = db().await;
    history(&pool, ORG, "B1").await;
    position(&pool, ORG, "B1", "USD", "60", "1921.00").await;
    assert_eq!(
        keys(&offenders(&pool, &PositionsCheck, &scope(None)).await),
        ["B1:USD"]
    );
    pool.close().await;
}

#[tokio::test]
async fn a_position_without_movements_is_clean_only_at_zero() {
    let pool = db().await;
    position(&pool, ORG, "B1", "EUR", "0", "0.00").await;
    position(&pool, ORG, "B2", "EUR", "5", "0.00").await;
    assert_eq!(
        keys(&offenders(&pool, &PositionsCheck, &scope(None)).await),
        ["B2:EUR"],
        "Σ of no movement is 0, so 0 / 0.00 is clean and 5 is not"
    );
    pool.close().await;
}

#[tokio::test]
async fn movements_of_another_branch_or_currency_do_not_count() {
    let pool = db().await;
    history(&pool, ORG, "B1").await;
    movement(&pool, ORG, "B2", "USD", "ACQUIRE", "7", "224.00").await;
    movement(&pool, ORG, "B1", "EUR", "ACQUIRE", "3", "110.00").await;
    position(&pool, ORG, "B1", "USD", "60", "1920.00").await;
    assert_eq!(
        offenders(&pool, &PositionsCheck, &scope(None)).await,
        vec![]
    );
    pool.close().await;
}

#[tokio::test]
async fn the_scope_filters_by_branch_and_by_organisation() {
    let pool = db().await;
    history(&pool, ORG, "B1").await;
    position(&pool, ORG, "B1", "USD", "61", "1920.00").await;
    history(&pool, OTHER_ORG, "X1").await;
    position(&pool, OTHER_ORG, "X1", "USD", "61", "1920.00").await;
    assert_eq!(
        keys(&offenders(&pool, &PositionsCheck, &scope(Some("B2"))).await),
        Vec::<String>::new()
    );
    assert_eq!(
        keys(&offenders(&pool, &PositionsCheck, &scope(Some("B1"))).await),
        ["B1:USD"]
    );
    assert_eq!(
        keys(&offenders(&pool, &PositionsCheck, &scope(None)).await),
        ["B1:USD"]
    );
    pool.close().await;
}

#[tokio::test]
async fn more_than_100_offenders_give_100_and_the_marker() {
    let pool = db().await;
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id) \
         SELECT 'BULK' || n, 'BULK' || n, $1::uuid FROM generate_series(1, 101) n",
    )
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("101 branches");
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, \
             version) SELECT $1::uuid, 'BULK' || n, 'USD', 1, 0, 1 FROM generate_series(1, 101) n",
    )
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("101 positions without movements");
    assert_capped(&offenders(&pool, &PositionsCheck, &scope(None)).await);
    pool.close().await;
}

#[tokio::test]
async fn query_is_one_capped_statement_and_the_check_is_registered() {
    assert!(!QUERY.trim().is_empty(), "QUERY holds the statement");
    assert!(
        QUERY.to_uppercase().contains("LIMIT 101"),
        "the cap is in the statement: {QUERY}"
    );
    let pool = db().await;
    assert_registered(&pool, 2).await;
    pool.close().await;
}
