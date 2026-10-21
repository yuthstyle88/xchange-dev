//! Up-front tests for LG-7.3 (#1057; design `docs/design/blueprint/ledger-inventory.md` §6 check 3,
//! §6.1): per branch and foreign currency, Σ balances of that branch's own DRAWER, SAFE and
//! IN_TRANSIT accounts equals the branch's inventory position quantity. Written by the ticket
//! author before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it
//! does not skip. Broken postings are stored with triggers off (`session_replication_role = replica`),
//! as a hand-edited row would be.

use xc_cash::application::reconcile::places::{PlacesCheck, QUERY};

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_cash::application::reconcile::{Offender, Reconcile, ReconcileCheck, ReconcileScope};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const OTHER_ORG: &str = "00000000-0000-0000-0000-0000000000f2";
const SYSTEM_USER: &str = "00000000-0000-0000-0000-0000000000aa";

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
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2::uuid, 'system', 'ACTIVE', 'test')",
    )
    .bind(SYSTEM_USER)
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("system user");
    pool
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a valid decimal")
}

fn fa(s: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(d(s))
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").expect("USD")
}

fn scope(branch: Option<&str>) -> ReconcileScope {
    ReconcileScope {
        organization_id: ORG.to_string(),
        branch: branch.map(BranchId::new),
        period: None,
    }
}

/// A cash account, inserted directly: answers its id as text.
async fn account(
    pool: &PgPool,
    org: &str,
    branch: &str,
    currency: &str,
    kind: &str,
    shift_id: Option<&str>,
) -> String {
    sqlx::query_scalar(
        "INSERT INTO cash_accounts (id, organization_id, branch_id, currency, kind, shift_id, \
             created_at) VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4, $5, now()) \
         RETURNING id::text",
    )
    .bind(org)
    .bind(branch)
    .bind(currency)
    .bind(kind)
    .bind(shift_id)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("account {branch} {currency} {kind}: {e}"))
}

async fn position(pool: &PgPool, org: &str, branch: &str, currency: &str, q: &str) {
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, \
             version) VALUES ($1::uuid, $2, $3, $4::numeric, 0, 1) \
         ON CONFLICT (organization_id, branch_id, currency) \
         DO UPDATE SET quantity = EXCLUDED.quantity",
    )
    .bind(org)
    .bind(branch)
    .bind(currency)
    .bind(q)
    .execute(pool)
    .await
    .expect("a position");
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

async fn seed_shift(pool: &PgPool, branch: &str) -> String {
    let shift_id = unique("shift");
    let teller_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'teller', 'ACTIVE', 'test')",
    )
    .bind(teller_id)
    .bind(ORG)
    .execute(pool)
    .await
    .expect("seed teller");

    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), 'user', $3, $4)",
    )
    .bind(&shift_id)
    .bind(branch)
    .bind(unique("req"))
    .bind(teller_id)
    .execute(pool)
    .await
    .expect("seed shift");
    shift_id
}

async fn seed_transfer(pool: &PgPool, org: &str, from_branch: &str, to_branch: &str) -> String {
    let transfer_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO cash_transfers (id, organization_id, kind, from_branch_id, to_branch_id, \
             status, created_by, idempotency_key, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, 'BRANCH', $3, $4, 'DISPATCHED', $5::uuid, $6, now(), now())",
    )
    .bind(&transfer_id)
    .bind(org)
    .bind(from_branch)
    .bind(to_branch)
    .bind(SYSTEM_USER)
    .bind(unique("transfer_key"))
    .execute(pool)
    .await
    .expect("seed transfer");
    transfer_id
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
    assert_eq!(
        found[100].detail, "more than 100 offenders; the first 100 are listed",
        "the marker detail matches: {:?}",
        found[100]
    );
    assert!(found[..100].iter().all(|o| o.key != "truncated"));
}

#[allow(dead_code)]
fn _uses(_: Arc<dyn ReconcileCheck>) {}

#[tokio::test]
async fn a_clean_ledger_after_an_exchange_is_clean() {
    let pool = db().await;
    let safe = account(&pool, ORG, "B1", "USD", "SAFE", None).await;
    let cust = account(&pool, ORG, "B1", "USD", "CUSTOMER", None).await;
    let safe_thb = account(&pool, ORG, "B1", "THB", "SAFE", None).await;
    let cust_thb = account(&pool, ORG, "B1", "THB", "CUSTOMER", None).await;
    post_raw(
        &pool,
        ORG,
        "B1",
        "EXCHANGE",
        &[
            (&safe, "USD", "100", "3500.00"),
            (&cust, "USD", "-100", "-3500.00"),
            (&safe_thb, "THB", "-3500", "-3500.00"),
            (&cust_thb, "THB", "3500", "3500.00"),
        ],
    )
    .await;
    position(&pool, ORG, "B1", "USD", "100").await;

    assert_eq!(offenders(&pool, &PlacesCheck, &scope(None)).await, vec![]);
    assert_eq!(PlacesCheck.id(), 3);
    pool.close().await;
}

#[tokio::test]
async fn a_branch_transfer_b1_to_b2_dispatched_has_no_offender() {
    let pool = db().await;
    let shift = seed_shift(&pool, "B1").await;
    let transfer = seed_transfer(&pool, ORG, "B1", "B2").await;

    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");

    // Acquire 100 USD into B1's DRAWER
    let src_open = SourceRef {
        source_type: SourceType::Opening,
        source_id: unique("src"),
        source_key: "open".to_string(),
    };
    let req_open = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: src_open,
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_open], None)
        .await
        .expect("acquire 100 USD into drawer");

    // Dispatch 100 USD from DRAWER to IN_TRANSIT
    let src_dispatch = SourceRef {
        source_type: SourceType::Transfer,
        source_id: transfer.clone(),
        source_key: "dispatch".to_string(),
    };
    let req_dispatch = PostingRequest {
        from: CashPlace::Drawer(shift),
        to: CashPlace::InTransit(transfer),
        currency: usd(),
        quantity: fa("100"),
        source: src_dispatch,
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_dispatch], None)
        .await
        .expect("dispatch 100 USD to in-transit");

    tx.commit().await.expect("commit");

    // B1's IN_TRANSIT holds 100 USD and B1's position is 100 USD -> clean!
    assert_eq!(offenders(&pool, &PlacesCheck, &scope(None)).await, vec![]);
    pool.close().await;
}

#[tokio::test]
async fn received_short_leaves_shortfall_on_b1_in_transit_and_position_with_no_offender() {
    let pool = db().await;
    let shift = seed_shift(&pool, "B1").await;
    let transfer = seed_transfer(&pool, ORG, "B1", "B2").await;

    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");

    // Acquire 100 USD into B1's DRAWER
    let src_open = SourceRef {
        source_type: SourceType::Opening,
        source_id: unique("src"),
        source_key: "open".to_string(),
    };
    let req_open = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: src_open,
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_open], None)
        .await
        .expect("acquire 100 USD into drawer");

    // Dispatch 100 USD to IN_TRANSIT
    let src_dispatch = SourceRef {
        source_type: SourceType::Transfer,
        source_id: transfer.clone(),
        source_key: "dispatch".to_string(),
    };
    let req_dispatch = PostingRequest {
        from: CashPlace::Drawer(shift),
        to: CashPlace::InTransit(transfer.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: src_dispatch,
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_dispatch], None)
        .await
        .expect("dispatch 100 USD to in-transit");

    // Receive 90 USD at B2 SAFE (10 USD remains in B1's IN_TRANSIT and position)
    let src_rcv = SourceRef {
        source_type: SourceType::Transfer,
        source_id: transfer.clone(),
        source_key: "receive".to_string(),
    };
    let req_rcv = PostingRequest {
        from: CashPlace::InTransit(transfer),
        to: CashPlace::Safe(BranchId::new("B2")),
        currency: usd(),
        quantity: fa("90"),
        source: src_rcv,
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_rcv], None)
        .await
        .expect("receive 90 USD at B2");

    tx.commit().await.expect("commit");

    // Both B1 (10 places vs 10 position) and B2 (90 places vs 90 position) match!
    assert_eq!(offenders(&pool, &PlacesCheck, &scope(None)).await, vec![]);
    pool.close().await;
}

#[tokio::test]
async fn after_reconcile_in_transit_to_variance_has_no_offender() {
    let pool = db().await;
    let shift = seed_shift(&pool, "B1").await;
    let transfer = seed_transfer(&pool, ORG, "B1", "B2").await;

    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");

    let src_open = SourceRef {
        source_type: SourceType::Opening,
        source_id: unique("src"),
        source_key: "open".to_string(),
    };
    let req_open = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: src_open,
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_open], None)
        .await
        .expect("acquire 100 USD into drawer");

    let src_dispatch = SourceRef {
        source_type: SourceType::Transfer,
        source_id: transfer.clone(),
        source_key: "dispatch".to_string(),
    };
    let req_dispatch = PostingRequest {
        from: CashPlace::Drawer(shift),
        to: CashPlace::InTransit(transfer.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: src_dispatch,
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_dispatch], None)
        .await
        .expect("dispatch 100 USD to in-transit");

    let src_rcv = SourceRef {
        source_type: SourceType::Transfer,
        source_id: transfer.clone(),
        source_key: "receive".to_string(),
    };
    let req_rcv = PostingRequest {
        from: CashPlace::InTransit(transfer),
        to: CashPlace::Safe(BranchId::new("B2")),
        currency: usd(),
        quantity: fa("90"),
        source: src_rcv,
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_rcv], None)
        .await
        .expect("receive 90 USD at B2");

    tx.commit().await.expect("commit");

    // Reconcile: the remaining 10 USD moves from IN_TRANSIT to VARIANCE
    let variance_acc = account(&pool, ORG, "B1", "USD", "VARIANCE", None).await;
    let transit_acc: String = sqlx::query_scalar(
        "SELECT id::text FROM cash_accounts \
         WHERE organization_id = $1::uuid AND branch_id = 'B1' AND kind = 'IN_TRANSIT' AND currency = 'USD'",
    )
    .bind(ORG)
    .fetch_one(&pool)
    .await
    .expect("in-transit account");

    post_raw(
        &pool,
        ORG,
        "B1",
        "ADJUSTMENT",
        &[
            (&transit_acc, "USD", "-10", "0.00"),
            (&variance_acc, "USD", "10", "0.00"),
        ],
    )
    .await;

    // Both places and position drop by 10 (position is now 0)
    sqlx::query(
        "UPDATE inventory_positions SET quantity = quantity - 10 \
         WHERE organization_id = $1::uuid AND branch_id = 'B1' AND currency = 'USD'",
    )
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("adjust position");

    // B1 places = 0, position = 0 -> clean!
    assert_eq!(offenders(&pool, &PlacesCheck, &scope(None)).await, vec![]);
    pool.close().await;
}

#[tokio::test]
async fn a_hand_inserted_entry_on_b1_safe_is_reported() {
    let pool = db().await;
    let safe = account(&pool, ORG, "B1", "USD", "SAFE", None).await;
    let cust = account(&pool, ORG, "B1", "USD", "CUSTOMER", None).await;
    position(&pool, ORG, "B1", "USD", "60").await;
    // Safe has 70 while position has 60
    post_raw(
        &pool,
        ORG,
        "B1",
        "ADJUSTMENT",
        &[(&safe, "USD", "70", "0.00"), (&cust, "USD", "-70", "0.00")],
    )
    .await;
    let found = offenders(&pool, &PlacesCheck, &scope(None)).await;
    assert_eq!(keys(&found), ["B1:USD"]);
    assert!(
        found[0].detail.contains("places 70") && found[0].detail.contains("position 60"),
        "the detail says what differs: {:?}",
        found[0].detail
    );
    pool.close().await;
}

#[tokio::test]
async fn a_hand_inserted_entry_without_position_is_reported() {
    let pool = db().await;
    let safe = account(&pool, ORG, "B1", "USD", "SAFE", None).await;
    let cust = account(&pool, ORG, "B1", "USD", "CUSTOMER", None).await;
    // Safe has 70 while no inventory_positions row exists
    post_raw(
        &pool,
        ORG,
        "B1",
        "ADJUSTMENT",
        &[(&safe, "USD", "70", "0.00"), (&cust, "USD", "-70", "0.00")],
    )
    .await;
    let found = offenders(&pool, &PlacesCheck, &scope(None)).await;
    assert_eq!(keys(&found), ["B1:USD"]);
    assert!(
        found[0].detail.contains("places 70") && found[0].detail.contains("position 0"),
        "the detail says what differs: {:?}",
        found[0].detail
    );
    pool.close().await;
}

#[tokio::test]
async fn thb_balances_that_differ_from_any_position_are_not_checked() {
    let pool = db().await;
    let safe = account(&pool, ORG, "B1", "THB", "SAFE", None).await;
    let cust = account(&pool, ORG, "B1", "THB", "CUSTOMER", None).await;
    post_raw(
        &pool,
        ORG,
        "B1",
        "ADJUSTMENT",
        &[
            (&safe, "THB", "50000", "50000.00"),
            (&cust, "THB", "-50000", "-50000.00"),
        ],
    )
    .await;
    // THB is not inventory: even without positions it is not reported
    assert_eq!(offenders(&pool, &PlacesCheck, &scope(None)).await, vec![]);
    pool.close().await;
}

#[tokio::test]
async fn the_scope_filters_by_branch_and_by_organisation() {
    let pool = db().await;
    let s_b1 = account(&pool, ORG, "B1", "USD", "SAFE", None).await;
    let c_b1 = account(&pool, ORG, "B1", "USD", "CUSTOMER", None).await;
    position(&pool, ORG, "B1", "USD", "60").await;
    post_raw(
        &pool,
        ORG,
        "B1",
        "ADJUSTMENT",
        &[(&s_b1, "USD", "70", "0.00"), (&c_b1, "USD", "-70", "0.00")],
    )
    .await;

    let s_b2 = account(&pool, ORG, "B2", "USD", "SAFE", None).await;
    let c_b2 = account(&pool, ORG, "B2", "USD", "CUSTOMER", None).await;
    position(&pool, ORG, "B2", "USD", "50").await;
    post_raw(
        &pool,
        ORG,
        "B2",
        "ADJUSTMENT",
        &[(&s_b2, "USD", "50", "0.00"), (&c_b2, "USD", "-50", "0.00")],
    )
    .await;

    let s_x1 = account(&pool, OTHER_ORG, "X1", "USD", "SAFE", None).await;
    let c_x1 = account(&pool, OTHER_ORG, "X1", "USD", "CUSTOMER", None).await;
    position(&pool, OTHER_ORG, "X1", "USD", "60").await;
    post_raw(
        &pool,
        OTHER_ORG,
        "X1",
        "ADJUSTMENT",
        &[(&s_x1, "USD", "80", "0.00"), (&c_x1, "USD", "-80", "0.00")],
    )
    .await;

    assert_eq!(
        keys(&offenders(&pool, &PlacesCheck, &scope(Some("B2"))).await),
        Vec::<String>::new()
    );
    assert_eq!(
        keys(&offenders(&pool, &PlacesCheck, &scope(Some("B1"))).await),
        ["B1:USD"]
    );
    assert_eq!(
        keys(&offenders(&pool, &PlacesCheck, &scope(None)).await),
        ["B1:USD"]
    );
    pool.close().await;
}

#[tokio::test]
async fn places_of_another_branch_do_not_count() {
    let pool = db().await;
    let s_b1 = account(&pool, ORG, "B1", "USD", "SAFE", None).await;
    let c_b1 = account(&pool, ORG, "B1", "USD", "CUSTOMER", None).await;
    position(&pool, ORG, "B1", "USD", "60").await;
    post_raw(
        &pool,
        ORG,
        "B1",
        "ADJUSTMENT",
        &[(&s_b1, "USD", "60", "0.00"), (&c_b1, "USD", "-60", "0.00")],
    )
    .await;

    let s_b2 = account(&pool, ORG, "B2", "USD", "SAFE", None).await;
    let c_b2 = account(&pool, ORG, "B2", "USD", "CUSTOMER", None).await;
    position(&pool, ORG, "B2", "USD", "15").await;
    post_raw(
        &pool,
        ORG,
        "B2",
        "ADJUSTMENT",
        &[(&s_b2, "USD", "15", "0.00"), (&c_b2, "USD", "-15", "0.00")],
    )
    .await;

    assert_eq!(offenders(&pool, &PlacesCheck, &scope(None)).await, vec![]);
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
    .expect("101 positions without places");
    assert_capped(&offenders(&pool, &PlacesCheck, &scope(None)).await);
    pool.close().await;
}

#[tokio::test]
async fn exactly_100_offenders_have_no_truncation_marker() {
    let pool = db().await;
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id) \
         SELECT 'BULK' || n, 'BULK' || n, $1::uuid FROM generate_series(1, 100) n",
    )
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("100 branches");
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, \
             version) SELECT $1::uuid, 'BULK' || n, 'USD', 1, 0, 1 FROM generate_series(1, 100) n",
    )
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("100 positions without places");
    let found = offenders(&pool, &PlacesCheck, &scope(None)).await;
    assert_eq!(found.len(), 100);
    assert!(
        found.iter().all(|o| o.key != "truncated"),
        "100 offenders must not have truncation marker"
    );
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
    assert_registered(&pool, 3).await;
    pool.close().await;
}
