//! Up-front tests for LG-1 (#923; design `docs/design/blueprint/ledger-inventory.md` §3.1–§3.3, §8):
//! migration 1, the balance trigger, and the two repositories, on real PostgreSQL. Written by the
//! ticket author before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it
//! does not skip. Each test gets a fresh schema with every migration of `api/migrations` applied.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_cash::domain::inventory::repository::{InventoryRepository, Lot, Movement, MovementType};
use xc_cash::domain::inventory::weighted_average::WeightedAverage;
use xc_cash::domain::ledger::account::{AccountKind, CashAccountId};
use xc_cash::domain::ledger::posting::{Entry, Posting, PostingId, SourceRef, SourceType};
use xc_cash::domain::ledger::repository::LedgerRepository;
use xc_cash::infrastructure::inventory::repository::PgInventoryRepository;
use xc_cash::infrastructure::ledger::repository::PgLedgerRepository;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 15, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").expect("USD")
}

fn thb() -> CurrencyCode {
    CurrencyCode::parse("THB").expect("THB")
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// A pool on a fresh schema with every migration applied, and a branch of the organisation.
async fn db() -> (PgPool, BranchId) {
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
    let schema = unique("lg1");
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
        .max_connections(6)
        .connect_with(options.options([("search_path", schema.as_str())]))
        .await
        .expect("connect to the test schema");
    Migrator::new(migrations_dir().as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies in version order on a fresh database");
    let branch = unique("B");
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(&branch)
        .execute(&pool)
        .await
        .expect("insert a branch");
    (pool, BranchId::new(branch))
}

fn source(id: &str) -> SourceRef {
    SourceRef {
        source_type: SourceType::Exchange,
        source_id: id.to_string(),
        source_key: "complete".to_string(),
    }
}

fn entry(account: CashAccountId, currency: CurrencyCode, q: &str, v: &str) -> Entry {
    Entry {
        account,
        currency,
        quantity_signed: d(q),
        valuation_thb: d(v),
    }
}

fn posting(branch: &BranchId, src: SourceRef, entries: Vec<Entry>) -> Posting {
    Posting {
        organization_id: ORG.to_string(),
        branch: branch.clone(),
        source: src,
        posted_at: at(0),
        posted_by: "test".to_string(),
        entries,
    }
}

/// The SAFE and CUSTOMER accounts of `branch` for `currency`, created in their own transaction.
async fn accounts(
    pool: &PgPool,
    branch: &BranchId,
    currency: CurrencyCode,
) -> (CashAccountId, CashAccountId) {
    let repo = PgLedgerRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    let safe = repo
        .account_in(
            &mut tx,
            ORG,
            Some(branch),
            currency,
            AccountKind::Safe,
            None,
        )
        .await
        .expect("the safe account");
    let customer = repo
        .account_in(
            &mut tx,
            ORG,
            Some(branch),
            currency,
            AccountKind::Customer,
            None,
        )
        .await
        .expect("the customer account");
    tx.commit().await.expect("commit the accounts");
    (safe, customer)
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("count {table}: {e}"))
}

#[tokio::test]
async fn a_balanced_posting_commits_and_balances_read_back() {
    let (pool, branch) = db().await;
    let (safe, customer) = accounts(&pool, &branch, usd()).await;
    let repo = PgLedgerRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    repo.insert_posting_in(
        &mut tx,
        &posting(
            &branch,
            source("tx-1"),
            vec![
                entry(safe, usd(), "100", "3200.00"),
                entry(customer, usd(), "-100", "-3200.00"),
            ],
        ),
    )
    .await
    .expect("insert the posting");
    tx.commit().await.expect("a balanced posting commits");

    let mut tx = pool.begin().await.expect("begin");
    assert_eq!(
        repo.balance_in(&mut tx, safe).await.expect("safe"),
        d("100")
    );
    assert_eq!(
        repo.balance_in(&mut tx, customer).await.expect("customer"),
        d("-100")
    );
    drop(tx);
    assert_eq!(count(&pool, "ledger_postings").await, 1);
    assert_eq!(count(&pool, "ledger_entries").await, 2);
    pool.close().await;
}

#[tokio::test]
async fn a_posting_unbalanced_in_one_currency_is_refused_at_commit_and_nothing_is_kept() {
    let (pool, branch) = db().await;
    let (safe_usd, customer_usd) = accounts(&pool, &branch, usd()).await;
    let (safe_thb, customer_thb) = accounts(&pool, &branch, thb()).await;
    let repo = PgLedgerRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    // THB balances, USD does not.
    let inserted = repo
        .insert_posting_in(
            &mut tx,
            &posting(
                &branch,
                source("tx-bad"),
                vec![
                    entry(safe_usd, usd(), "100", "3200.00"),
                    entry(customer_usd, usd(), "-99", "-3200.00"),
                    entry(safe_thb, thb(), "-3200", "-3200.00"),
                    entry(customer_thb, thb(), "3200", "3200.00"),
                ],
            ),
        )
        .await;
    let committed = match inserted {
        Ok(_) => tx.commit().await.is_ok(),
        Err(_) => {
            // Release the connection before `pool.close()`, or the close waits for it forever.
            drop(tx);
            false
        }
    };
    assert!(!committed, "the balance trigger refuses the posting");
    assert_eq!(count(&pool, "ledger_postings").await, 0, "nothing kept");
    assert_eq!(count(&pool, "ledger_entries").await, 0, "nothing kept");
    pool.close().await;
}

#[tokio::test]
async fn the_balance_is_checked_once_at_commit_not_per_statement() {
    let (pool, branch) = db().await;
    let (safe, customer) = accounts(&pool, &branch, usd()).await;
    let mut tx = pool.begin().await.expect("begin");
    let posting_id: String = sqlx::query_scalar(
        "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, \
             source_key, status, posted_at, posted_by) \
         VALUES (gen_random_uuid(), $1::uuid, $2, 'ADJUSTMENT', 'adj-1', 'k', 'POSTED', now(), 't') \
         RETURNING id::text",
    )
    .bind(ORG)
    .bind(branch.as_str())
    .fetch_one(&mut *tx)
    .await
    .expect("a posting row");
    for (account, q) in [(safe, "10"), (customer, "-10")] {
        // Two statements: after the first, the posting is unbalanced; the trigger must wait.
        sqlx::query(
            "INSERT INTO ledger_entries (posting_id, account_id, currency, quantity_signed, \
                 valuation_thb, created_at) \
             VALUES ($1::uuid, $2::uuid, 'USD', $3::numeric, 0, now())",
        )
        .bind(&posting_id)
        .bind(account.0.to_string())
        .bind(q)
        .execute(&mut *tx)
        .await
        .expect("an entry statement");
    }
    tx.commit().await.expect("balanced at commit");
    assert_eq!(count(&pool, "ledger_entries").await, 2);
    pool.close().await;
}

#[tokio::test]
async fn the_same_source_twice_is_one_posting() {
    let (pool, branch) = db().await;
    let (safe, customer) = accounts(&pool, &branch, usd()).await;
    let repo = PgLedgerRepository::new(pool.clone());
    let p = posting(
        &branch,
        source("tx-1"),
        vec![
            entry(safe, usd(), "5", "160.00"),
            entry(customer, usd(), "-5", "-160.00"),
        ],
    );
    let mut ids: Vec<PostingId> = Vec::new();
    for _ in 0..2 {
        let mut tx = pool.begin().await.expect("begin");
        ids.push(repo.insert_posting_in(&mut tx, &p).await.expect("insert"));
        tx.commit().await.expect("commit");
    }
    assert_eq!(ids[0], ids[1], "the second call answers the first id");
    assert_eq!(count(&pool, "ledger_postings").await, 1);
    assert_eq!(
        count(&pool, "ledger_entries").await,
        2,
        "no entry written twice"
    );
    pool.close().await;
}

#[tokio::test]
async fn account_in_finds_or_creates_one_row_per_key() {
    let (pool, branch) = db().await;
    let repo = PgLedgerRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    let a = repo
        .account_in(&mut tx, ORG, Some(&branch), usd(), AccountKind::Safe, None)
        .await
        .expect("first");
    let b = repo
        .account_in(&mut tx, ORG, Some(&branch), usd(), AccountKind::Safe, None)
        .await
        .expect("again");
    let other = repo
        .account_in(&mut tx, ORG, Some(&branch), thb(), AccountKind::Safe, None)
        .await
        .expect("another currency");
    let org_wide = repo
        .account_in(&mut tx, ORG, None, usd(), AccountKind::Bank, None)
        .await
        .expect("an organisation-wide account");
    let org_wide_again = repo
        .account_in(&mut tx, ORG, None, usd(), AccountKind::Bank, None)
        .await
        .expect("again, with NULL branch and shift");
    tx.commit().await.expect("commit");
    assert_eq!(a, b);
    assert_ne!(a, other);
    assert_eq!(org_wide, org_wide_again, "NULLs are not distinct");
    assert_eq!(count(&pool, "cash_accounts").await, 3);
    pool.close().await;
}

#[tokio::test]
async fn two_account_in_for_one_key_at_once_make_one_row() {
    let (pool, branch) = db().await;
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let pool = pool.clone();
        let branch = branch.clone();
        tasks.push(tokio::spawn(async move {
            let repo = PgLedgerRepository::new(pool.clone());
            let mut tx = pool.begin().await.expect("begin");
            let id = repo
                .account_in(&mut tx, ORG, Some(&branch), usd(), AccountKind::Safe, None)
                .await
                .expect("account_in");
            tx.commit().await.expect("commit");
            id
        }));
    }
    let mut ids = Vec::new();
    for t in tasks {
        ids.push(t.await.expect("the task"));
    }
    assert!(ids.windows(2).all(|w| w[0] == w[1]), "one id: {ids:?}");
    assert_eq!(count(&pool, "cash_accounts").await, 1);
    pool.close().await;
}

#[tokio::test]
async fn an_entry_with_zero_quantity_is_refused() {
    let (pool, branch) = db().await;
    let (safe, customer) = accounts(&pool, &branch, usd()).await;
    let repo = PgLedgerRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    let inserted = repo
        .insert_posting_in(
            &mut tx,
            &posting(
                &branch,
                source("tx-zero"),
                vec![
                    entry(safe, usd(), "0", "0"),
                    entry(customer, usd(), "0", "0"),
                ],
            ),
        )
        .await;
    let committed = match inserted {
        Ok(_) => tx.commit().await.is_ok(),
        Err(_) => {
            // Release the connection before `pool.close()`, or the close waits for it forever.
            drop(tx);
            false
        }
    };
    assert!(!committed, "quantity_signed <> 0");
    assert_eq!(count(&pool, "ledger_entries").await, 0);
    pool.close().await;
}

#[tokio::test]
async fn a_drawer_needs_a_shift_and_only_a_drawer_has_one() {
    let (pool, branch) = db().await;
    let ok = sqlx::query(
        "INSERT INTO cash_accounts (id, organization_id, branch_id, currency, kind, shift_id, \
             created_at) VALUES (gen_random_uuid(), $1::uuid, $2, 'USD', 'SAFE', NULL, now())",
    )
    .bind(ORG)
    .bind(branch.as_str())
    .execute(&pool)
    .await;
    assert!(ok.is_ok(), "a SAFE without a shift is a valid row: {ok:?}");
    for (kind, shift) in [("DRAWER", None), ("SAFE", Some("shift-1"))] {
        let refused = sqlx::query(
            "INSERT INTO cash_accounts (id, organization_id, branch_id, currency, kind, shift_id, \
                 created_at) VALUES (gen_random_uuid(), $1::uuid, $2, 'USD', $3, $4, now())",
        )
        .bind(ORG)
        .bind(branch.as_str())
        .bind(kind)
        .bind(shift)
        .execute(&pool)
        .await;
        assert!(refused.is_err(), "{kind} with shift {shift:?} is refused");
    }
    pool.close().await;
}

#[tokio::test]
async fn cash_limits_takes_null_or_a_non_negative_limit() {
    let (pool, branch) = db().await;
    let insert = |limit: Option<&'static str>| {
        let pool = pool.clone();
        let branch = branch.clone();
        async move {
            sqlx::query(
                "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, \
                     updated_by, updated_at) VALUES ($1::uuid, $2, $3::numeric, 't', now()) \
                 ON CONFLICT (organization_id, branch_id) \
                 DO UPDATE SET count_difference_limit_thb = EXCLUDED.count_difference_limit_thb",
            )
            .bind(ORG)
            .bind(branch.as_str())
            .bind(limit)
            .execute(&pool)
            .await
        }
    };
    assert!(
        insert(None).await.is_ok(),
        "NULL: every difference goes to FINANCE"
    );
    assert!(insert(Some("0")).await.is_ok());
    assert!(insert(Some("500.00")).await.is_ok());
    assert!(
        insert(Some("-1")).await.is_err(),
        "a negative limit is refused"
    );
    pool.close().await;
}

#[tokio::test]
async fn record_in_moves_the_position_and_position_for_update_reads_it() {
    let (pool, branch) = db().await;
    let (safe, customer) = accounts(&pool, &branch, usd()).await;
    let ledger = PgLedgerRepository::new(pool.clone());
    let inventory = PgInventoryRepository::new(pool.clone());

    let mut tx = pool.begin().await.expect("begin");
    assert_eq!(
        inventory
            .position_for_update_in(&mut tx, ORG, &branch, usd())
            .await
            .expect("no position yet"),
        WeightedAverage::EMPTY
    );
    let buy = ledger
        .insert_posting_in(
            &mut tx,
            &posting(
                &branch,
                source("buy-1"),
                vec![
                    entry(safe, usd(), "100", "3200.00"),
                    entry(customer, usd(), "-100", "-3200.00"),
                ],
            ),
        )
        .await
        .expect("the buy posting");
    let lot = Lot {
        id: Uuid::new_v4(),
        organization_id: ORG.to_string(),
        branch: branch.clone(),
        currency: usd(),
        denomination_id: None,
        acquired_at: at(0),
        quantity: d("100"),
        unit_cost_thb: d("32.00000000"),
        source_posting_id: buy,
    };
    inventory
        .record_in(
            &mut tx,
            &Movement {
                organization_id: ORG.to_string(),
                branch: branch.clone(),
                currency: usd(),
                lot_id: Some(lot.id),
                posting_id: buy,
                transaction_line_id: None,
                movement_type: MovementType::Acquire,
                quantity_signed: d("100"),
                cost_thb_signed: d("3200.00"),
                average_after: Some(d("32.00000000")),
                created_at: at(0),
            },
            Some(&lot),
        )
        .await
        .expect("record the acquisition");
    let sell = ledger
        .insert_posting_in(
            &mut tx,
            &posting(
                &branch,
                source("sell-1"),
                vec![
                    entry(customer, usd(), "40", "1280.00"),
                    entry(safe, usd(), "-40", "-1280.00"),
                ],
            ),
        )
        .await
        .expect("the sell posting");
    inventory
        .record_in(
            &mut tx,
            &Movement {
                organization_id: ORG.to_string(),
                branch: branch.clone(),
                currency: usd(),
                lot_id: None,
                posting_id: sell,
                transaction_line_id: None,
                movement_type: MovementType::Dispose,
                quantity_signed: d("-40"),
                cost_thb_signed: d("-1280.00"),
                average_after: Some(d("32.00000000")),
                created_at: at(1),
            },
            None,
        )
        .await
        .expect("record the disposal");
    assert_eq!(
        inventory
            .position_for_update_in(&mut tx, ORG, &branch, usd())
            .await
            .expect("the position"),
        WeightedAverage {
            quantity: d("60"),
            cost_thb: d("1920.00")
        }
    );
    tx.commit().await.expect("commit");

    let (lots, movements, version): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM inventory_lots), (SELECT count(*) FROM inventory_movements), \
                (SELECT version FROM inventory_positions WHERE currency = 'USD')",
    )
    .fetch_one(&pool)
    .await
    .expect("read back");
    assert_eq!(
        (lots, movements, version),
        (1, 2, 2),
        "one lot, two movements, version 2"
    );
    pool.close().await;
}

#[tokio::test]
async fn balance_is_per_currency_not_in_total() {
    let (pool, branch) = db().await;
    let (safe_usd, _) = accounts(&pool, &branch, usd()).await;
    let (_, customer_thb) = accounts(&pool, &branch, thb()).await;
    let repo = PgLedgerRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    // +100 USD against -100 THB: zero in total, unbalanced in each currency.
    let inserted = repo
        .insert_posting_in(
            &mut tx,
            &posting(
                &branch,
                source("tx-mixed"),
                vec![
                    entry(safe_usd, usd(), "100", "3200.00"),
                    entry(customer_thb, thb(), "-100", "-3200.00"),
                ],
            ),
        )
        .await;
    let committed = match inserted {
        Ok(_) => tx.commit().await.is_ok(),
        Err(_) => {
            drop(tx);
            false
        }
    };
    assert!(!committed, "each currency must balance on its own");
    assert_eq!(count(&pool, "ledger_postings").await, 0, "nothing kept");
    pool.close().await;
}
