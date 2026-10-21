//! Up-front tests for LG-7.1 (#1055; design `docs/design/blueprint/ledger-inventory.md` §6 check 1,
//! §6.1): every posting balances per currency. Written by the ticket author before the
//! implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it
//! does not skip. Broken postings are stored with triggers off (`session_replication_role = replica`),
//! as a hand-edited row would be.

use xc_cash::application::reconcile::balanced::{BalancedCheck, QUERY};

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

/// Safe and customer accounts of `branch` in `currency`.
async fn pair(pool: &PgPool, org: &str, branch: &str, currency: &str) -> (String, String) {
    (
        account(pool, org, branch, currency, "SAFE").await,
        account(pool, org, branch, currency, "CUSTOMER").await,
    )
}

#[tokio::test]
async fn a_clean_ledger_has_no_offender() {
    let pool = db().await;
    let (su, cu) = pair(&pool, ORG, "B1", "USD").await;
    let (st, ct) = pair(&pool, ORG, "B1", "THB").await;
    post_raw(
        &pool,
        ORG,
        "B1",
        "EXCHANGE",
        &[
            (&su, "USD", "100", "3200"),
            (&cu, "USD", "-100", "-3200"),
            (&st, "THB", "-3200", "-3200"),
            (&ct, "THB", "3200", "3200"),
        ],
    )
    .await;
    post_raw(
        &pool,
        ORG,
        "B1",
        "OPENING",
        &[(&su, "USD", "50", "1600"), (&cu, "USD", "-50", "-1600")],
    )
    .await;
    assert_eq!(offenders(&pool, &BalancedCheck, &scope(None)).await, vec![]);
    assert_eq!(BalancedCheck.id(), 1);
    pool.close().await;
}

#[tokio::test]
async fn a_posting_whose_usd_sums_to_one_is_reported_with_its_sum() {
    let pool = db().await;
    let (su, cu) = pair(&pool, ORG, "B1", "USD").await;
    let bad = post_raw(
        &pool,
        ORG,
        "B1",
        "EXCHANGE",
        &[(&su, "USD", "100", "3200"), (&cu, "USD", "-99", "-3200")],
    )
    .await;
    let found = offenders(&pool, &BalancedCheck, &scope(None)).await;
    assert_eq!(keys(&found), [format!("{bad}:USD")]);
    assert_eq!(
        d(found[0].detail.trim()),
        d("1"),
        "the detail is the non-zero sum: {:?}",
        found[0]
    );
    pool.close().await;
}

#[tokio::test]
async fn only_the_unbalanced_currency_of_a_posting_is_reported() {
    let pool = db().await;
    let (su, cu) = pair(&pool, ORG, "B1", "USD").await;
    let (st, ct) = pair(&pool, ORG, "B1", "THB").await;
    let bad = post_raw(
        &pool,
        ORG,
        "B1",
        "EXCHANGE",
        &[
            (&su, "USD", "100", "3200"),
            (&cu, "USD", "-100", "-3200"),
            (&st, "THB", "-3200", "-3200"),
            (&ct, "THB", "3100", "3200"),
        ],
    )
    .await;
    let found = offenders(&pool, &BalancedCheck, &scope(None)).await;
    assert_eq!(keys(&found), [format!("{bad}:THB")]);
    assert_eq!(d(found[0].detail.trim()), d("-100"));
    pool.close().await;
}

#[tokio::test]
async fn the_scope_filters_by_branch_and_by_organisation() {
    let pool = db().await;
    let (su, cu) = pair(&pool, ORG, "B1", "USD").await;
    let bad = post_raw(
        &pool,
        ORG,
        "B1",
        "EXCHANGE",
        &[(&su, "USD", "1", "32"), (&cu, "USD", "-2", "-32")],
    )
    .await;
    let (xs, xc) = pair(&pool, OTHER_ORG, "X1", "USD").await;
    post_raw(
        &pool,
        OTHER_ORG,
        "X1",
        "EXCHANGE",
        &[(&xs, "USD", "1", "32"), (&xc, "USD", "-2", "-32")],
    )
    .await;
    assert_eq!(
        keys(&offenders(&pool, &BalancedCheck, &scope(Some("B2"))).await),
        Vec::<String>::new(),
        "B2 has none"
    );
    assert_eq!(
        keys(&offenders(&pool, &BalancedCheck, &scope(Some("B1"))).await),
        [format!("{bad}:USD")]
    );
    assert_eq!(
        keys(&offenders(&pool, &BalancedCheck, &scope(None)).await),
        [format!("{bad}:USD")],
        "the other organisation's posting is not this organisation's"
    );
    pool.close().await;
}

#[tokio::test]
async fn more_than_100_offenders_give_100_and_the_marker() {
    let pool = db().await;
    let (su, cu) = pair(&pool, ORG, "B1", "USD").await;
    for _ in 0..101 {
        post_raw(
            &pool,
            ORG,
            "B1",
            "ADJUSTMENT",
            &[(&su, "USD", "1", "0"), (&cu, "USD", "-2", "0")],
        )
        .await;
    }
    assert_capped(&offenders(&pool, &BalancedCheck, &scope(None)).await);
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
    assert_registered(&pool, 1).await;
    pool.close().await;
}
