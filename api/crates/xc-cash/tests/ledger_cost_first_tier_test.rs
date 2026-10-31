//! Up-front tests for R2 (#564 issuecomment-6542441849, owner ruling *"ทำ T1"*): the provisional cost of stock with
//! no cost history reads the branch's current publication at its **first tier** (`tier_lower_exclusive_thb IS NULL`,
//! `rate-feed-connector.md` R-10), not only a `value_tier = 'ALL'` line. A MANUAL approve publishes tier lines only
//! (`pricing-tiers.md` §15.3 step 6, `:908`), so before this a count overage or a short sale with no history was
//! refused `missing_buy_rate` on every MANUAL branch.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add
//! tests in a file of your own. The harness is copied from `ledger_cost_no_history_test.rs` (each xc-cash test file
//! carries its own).
//!
//! Rows:
//! - T1: a count overage into a position with no history, current publication tier-only (T1…T4): the ADJUST is
//!   costed at T1's BUY, never T2–T4's.
//! - T2: an approved NEGATIVE_STOCK short sale with no history, same publication: provisional cost at T1's BUY.
//! - T3: tier-only publication whose T1 has no BUY price: refused naming the currency; no other tier is used.
//! - T4: kept: an untiered publication still costs at its `ALL` line (as `ledger_cost_no_history_test` R1).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use pos_error::AppError;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Connection, PgPool, Row, Transaction};
use uuid::Uuid;

use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, LineCost, PostingRequest, TransactionLineId,
};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, MoneyDelta};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a valid decimal")
}

fn fa(s: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(d(s))
}

fn money(s: &str) -> Money {
    Money::parse(s).expect("a valid money")
}

fn delta(s: &str) -> MoneyDelta {
    let minor = (d(s) * Decimal::from(100)).round().to_i128().expect("fits");
    MoneyDelta::from_minor(minor)
}

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).unwrap_or_else(|_| panic!("a currency code: {s}"))
}

fn usd() -> CurrencyCode {
    code("USD")
}

fn thb() -> CurrencyCode {
    code("THB")
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

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
    let schema = unique("firsttier");
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
    sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
        .bind(&branch)
        .execute(&pool)
        .await
        .expect("insert ops_state");

    (pool, BranchId::new(branch))
}

async fn seed_shift(pool: &PgPool, branch: &BranchId) -> String {
    let teller_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'test')",
    )
    .bind(teller_id)
    .bind(ORG)
    .execute(pool)
    .await
    .expect("insert user");
    let shift_id = unique("shift");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
    )
    .bind(&shift_id)
    .bind(branch.as_str())
    .bind(unique("open-req"))
    .bind(teller_id)
    .execute(pool)
    .await
    .expect("insert shift");
    shift_id
}

/// A SELL `transactions` row with one `transaction_lines` row (SQL: xc-exchange is not a dependency of
/// xc-cash, as in `ledger_postings_test.rs`).
async fn seed_sell_line(
    pool: &PgPool,
    branch: &BranchId,
    shift_id: &str,
    quantity: &str,
    thb_amount: &str,
) -> TransactionLineId {
    let cust_id = unique("CU");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $3, 'Somchai', 'LIVE', now())",
    )
    .bind(&cust_id)
    .bind(ORG)
    .bind(unique("CUSTNO"))
    .execute(pool)
    .await
    .expect("seed customer");
    let tx_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, \
         submission_id, payload_hash, customer_id, customer_name, nationality, \
         staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by, status, \
         completed_at) \
         VALUES ($1, $2::uuid, $3, $4, 'LIVE', $5, $6, 'hash', $7, 'Somchai', 'THA', \
         'system', 'WALK-IN', '', current_date, now(), now(), 'system', 'COMPLETED', now())",
    )
    .bind(tx_id)
    .bind(ORG)
    .bind(unique("DOC"))
    .bind(branch.as_str())
    .bind(shift_id)
    .bind(unique("sub"))
    .bind(&cust_id)
    .execute(pool)
    .await
    .expect("insert transaction");
    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
         quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, cost_basis_thb, \
         realized_margin_thb, line_no, created_at, updated_at) \
         VALUES ($1, $2::uuid, $3, 'SELL', 'USD', $4, 1, 1.0, $5, 0, $5, NULL, NULL, 1, now(), now())",
    )
    .bind(line_id)
    .bind(ORG)
    .bind(tx_id)
    .bind(d(quantity))
    .bind(d(thb_amount))
    .execute(pool)
    .await
    .expect("insert transaction line");
    line_id
}

/// An APPROVED `NEGATIVE_STOCK` request for the line's transaction, at the hash the implementation
/// computes now (as `ledger_postings_test.rs` `approval`). Answers (approval, the transaction id).
async fn approved_negative_stock(
    pool: &PgPool,
    postings: &LedgerCashPostings,
    branch: &BranchId,
    line: TransactionLineId,
) -> (ApprovalRequestId, String) {
    let object_id: String =
        sqlx::query_scalar("SELECT transaction_id::text FROM transaction_lines WHERE id = $1")
            .bind(line)
            .fetch_one(pool)
            .await
            .expect("the line's transaction");
    let repo = PgApprovalRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    let hash = NegativeStockSource::version_hash(postings, &mut tx, &object_id)
        .await
        .expect("the source's version hash");
    let req = repo
        .insert_request(
            &mut tx,
            ORG,
            "NEGATIVE_STOCK",
            Some(branch.as_str().to_string()),
            &object_id,
            &hash,
            "v1",
            serde_json::json!({}),
            "teller",
            &unique("key"),
        )
        .await
        .expect("insert approval request");
    repo.update_status(&mut tx, &req.id, "APPROVED")
        .await
        .expect("approve the request");
    tx.commit().await.expect("commit");
    (ApprovalRequestId::new(req.id.as_str()), object_id)
}

/// One published line: (currency, denomination_group, value_tier, unit_size, buy_rate, sell_rate,
/// tier_lower_exclusive_thb, tier_upper_inclusive_thb).
type Line<'a> = (
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    Option<&'a str>,
    &'a str,
    Option<&'a str>,
    Option<&'a str>,
);

/// A rate publication on `branch` with `lines` (inserted in the order given), published `days_ago`
/// days before now. With `current`, `branches.current_publication_id` points at it.
async fn publish(
    pool: &PgPool,
    branch: &BranchId,
    days_ago: i32,
    lines: &[Line<'_>],
    current: bool,
) -> Uuid {
    let request = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES ($1, 'RATE_PUBLICATION', $2, $3, 'h', 'p', '{}'::jsonb, 'test', now(), 'APPROVED', $4, '00000000-0000-0000-0000-000000000001')",
    )
    .bind(request)
    .bind(branch.as_str())
    .bind(unique("pub-object"))
    .bind(unique("pub-key"))
    .execute(pool)
    .await
    .expect("an approved rate publication request");
    let publication = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
         VALUES ($1, $2, (SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $2), \
                 now() - make_interval(days => $3), $4, 'test')",
    )
    .bind(publication)
    .bind(branch.as_str())
    .bind(days_ago)
    .bind(request)
    .execute(pool)
    .await
    .expect("a rate publication");
    for (cur, group, tier, unit, buy, sell, lo, hi) in lines {
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, \
             value_tier, unit_size, buy_rate, sell_rate, source_snapshot, tier_lower_exclusive_thb, \
             tier_upper_inclusive_thb) \
             VALUES ($1, $2, $3, $4, $5, $6::numeric, $7::numeric, $8::numeric, '{}'::jsonb, \
                     $9::numeric, $10::numeric)",
        )
        .bind(Uuid::new_v4())
        .bind(publication)
        .bind(*cur)
        .bind(*group)
        .bind(*tier)
        .bind(*unit)
        .bind(*buy)
        .bind(*sell)
        .bind(*lo)
        .bind(*hi)
        .execute(pool)
        .await
        .expect("a published rate line");
    }
    if current {
        sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
            .bind(publication)
            .bind(branch.as_str())
            .execute(pool)
            .await
            .expect("make the publication current");
    }
    publication
}

/// The plain line: denomination group ALL, value tier ALL, no tier bounds.
fn all_line<'a>(cur: &'a str, unit: &'a str, buy: Option<&'a str>, sell: &'a str) -> Line<'a> {
    (cur, "ALL", "ALL", unit, buy, sell, None, None)
}

fn count_source() -> SourceRef {
    SourceRef {
        source_type: SourceType::Count,
        source_id: unique("count"),
        source_key: "difference".to_string(),
    }
}

fn exchange_source(id: &str) -> SourceRef {
    SourceRef {
        source_type: SourceType::Exchange,
        source_id: id.to_string(),
        source_key: "complete".to_string(),
    }
}

/// A count overage: `VARIANCE → DRAWER` (`adjust_in`, §4.4 row 7).
fn overage(branch: &BranchId, shift_id: &str, currency: CurrencyCode, qty: &str) -> PostingRequest {
    PostingRequest {
        from: CashPlace::Variance(branch.clone()),
        to: CashPlace::Drawer(shift_id.to_string()),
        currency,
        quantity: fa(qty),
        source: count_source(),
        line: None,
        reverses: None,
    }
}

fn sell_legs(
    shift_id: &str,
    line: TransactionLineId,
    qty: &str,
    thb_in: &str,
    src: SourceRef,
) -> [PostingRequest; 2] {
    [
        PostingRequest {
            from: CashPlace::Drawer(shift_id.to_string()),
            to: CashPlace::Customer,
            currency: usd(),
            quantity: fa(qty),
            source: src.clone(),
            line: Some(line),
            reverses: None,
        },
        PostingRequest {
            from: CashPlace::Customer,
            to: CashPlace::Drawer(shift_id.to_string()),
            currency: thb(),
            quantity: fa(thb_in),
            source: src,
            line: None,
            reverses: None,
        },
    ]
}

fn buy_legs(shift_id: &str, qty: &str, thb_out: &str, src: SourceRef) -> [PostingRequest; 2] {
    [
        PostingRequest {
            from: CashPlace::Customer,
            to: CashPlace::Drawer(shift_id.to_string()),
            currency: usd(),
            quantity: fa(qty),
            source: src.clone(),
            line: Some(Uuid::new_v4()),
            reverses: None,
        },
        PostingRequest {
            from: CashPlace::Drawer(shift_id.to_string()),
            to: CashPlace::Customer,
            currency: thb(),
            quantity: fa(thb_out),
            source: src,
            line: None,
            reverses: None,
        },
    ]
}

/// A THB deposit into `BANK`, `BANK → SAFE`, then `SAFE → DRAWER`, so the drawer can pay `amount` THB
/// (§4.5). THB legs move at face value and read no rate (row K3).
async fn fund_drawer_thb(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    postings: &LedgerCashPostings,
    branch: &BranchId,
    shift_id: &str,
    amount: &str,
) {
    let src = SourceRef {
        source_type: SourceType::Transfer,
        source_id: unique("fund-safe"),
        source_key: "external-in".to_string(),
    };
    let leg = |from: CashPlace, to: CashPlace, src: SourceRef| PostingRequest {
        from,
        to,
        currency: thb(),
        quantity: fa(amount),
        source: src,
        line: None,
        reverses: None,
    };
    postings
        .post_in(
            tx,
            &[
                leg(CashPlace::Customer, CashPlace::Bank, src.clone()),
                leg(CashPlace::Bank, CashPlace::Safe(branch.clone()), src),
            ],
            None,
        )
        .await
        .expect("fund: THB deposit into BANK and BANK -> SAFE");
    let open = SourceRef {
        source_type: SourceType::Opening,
        source_id: unique("fund-open"),
        source_key: "opening".to_string(),
    };
    postings
        .post_in(
            tx,
            &[leg(
                CashPlace::Safe(branch.clone()),
                CashPlace::Drawer(shift_id.to_string()),
                open,
            )],
            None,
        )
        .await
        .expect("fund: SAFE -> DRAWER");
}

/// The branch's movements of `currency`, oldest first: (type, quantity, cost, average_after).
async fn movements(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    currency: &str,
) -> Vec<(String, Decimal, Decimal, Option<Decimal>)> {
    sqlx::query_as(
        "SELECT movement_type, quantity_signed, cost_thb_signed, average_after FROM inventory_movements \
         WHERE organization_id = $1::uuid AND branch_id = $2 AND trim(currency) = $3 ORDER BY id",
    )
    .bind(ORG)
    .bind(branch.as_str())
    .bind(currency)
    .fetch_all(&mut **tx)
    .await
    .expect("read movements")
}

async fn position(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    currency: &str,
) -> (Decimal, Decimal) {
    sqlx::query_as(
        "SELECT quantity, cost_thb FROM inventory_positions \
         WHERE organization_id = $1::uuid AND branch_id = $2 AND trim(currency) = $3",
    )
    .bind(ORG)
    .bind(branch.as_str())
    .bind(currency)
    .fetch_one(&mut **tx)
    .await
    .expect("the position")
}

/// What a posting writes: postings, entries, movements, lots, and every position's figures.
async fn written(tx: &mut Transaction<'_, sqlx::Postgres>) -> (i64, i64, i64, i64, String) {
    let row = sqlx::query(
        "SELECT (SELECT count(*) FROM ledger_postings) AS p, (SELECT count(*) FROM ledger_entries) AS e, \
         (SELECT count(*) FROM inventory_movements) AS m, (SELECT count(*) FROM inventory_lots) AS l, \
         (SELECT coalesce(string_agg(currency || ':' || quantity::text || ':' || cost_thb::text, ',' \
          ORDER BY currency), '') FROM inventory_positions) AS pos",
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count what is written");
    (
        row.get("p"),
        row.get("e"),
        row.get("m"),
        row.get("l"),
        row.get("pos"),
    )
}

/// The posting is refused `validationFailed` naming `currency`, and writes nothing (rows R3, R4).
/// Runs in a savepoint.
async fn assert_refused_naming(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    postings: &LedgerCashPostings,
    reqs: &[PostingRequest],
    allow_negative: Option<ApprovalRequestId>,
    currency: &str,
    what: &str,
) {
    let mut sp = Connection::begin(&mut **tx).await.expect("savepoint");
    let before = written(&mut sp).await;
    match postings.post_in(&mut sp, reqs, allow_negative).await {
        Err(AppError::Domain(msg))
            if msg.starts_with("validationFailed") && msg.contains(currency) => {}
        other => panic!(
            "{what}: expected a validationFailed refusal naming {currency} (no published BUY rate), \
             got {other:?}"
        ),
    }
    assert_eq!(written(&mut sp).await, before, "{what}: nothing is written");
    sp.rollback().await.expect("rollback savepoint");
}

// ─────────────────────────────────────────────────────────────────────────────
// New behaviour: no cost history → the current published BUY rate
// ─────────────────────────────────────────────────────────────────────────────

/// V.9's fixed bands with BUY 32.50 / 32.60 / 32.70 / 32.80 (the first tier is the cheapest, so a reader that picks
/// any other tier gives a different figure). `lower_exclusive` NULL marks the first tier.
fn usd_tiers<'a>(t1_buy: Option<&'a str>) -> [Line<'a>; 4] {
    [
        (
            "USD",
            "ALL",
            "T1",
            "1",
            t1_buy,
            "33.40",
            None,
            Some("10000.00"),
        ),
        (
            "USD",
            "ALL",
            "T2",
            "1",
            Some("32.60"),
            "33.30",
            Some("10000.00"),
            Some("50000.00"),
        ),
        (
            "USD",
            "ALL",
            "T3",
            "1",
            Some("32.70"),
            "33.20",
            Some("50000.00"),
            Some("200000.00"),
        ),
        (
            "USD",
            "ALL",
            "T4",
            "1",
            Some("32.80"),
            "33.10",
            Some("200000.00"),
            None,
        ),
    ]
}

/// T1: an overage of +100 USD with no history, publication tier-only: ADJUST at T1's BUY 32.50.
#[tokio::test]
async fn t1_overage_with_no_history_on_a_tier_only_publication_is_costed_at_tier_1() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(&pool, &branch, 0, &usd_tiers(Some("32.50")), true).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");

    postings
        .post_in(&mut tx, &[overage(&branch, &shift_id, usd(), "100")], None)
        .await
        .expect("T1: an overage posts when the tier-only publication has a T1 BUY (main refuses missing_buy_rate)");

    // unit = round8(32.50 / 1); cost = round2(100 × 32.50) = 3250.00 (T2 3260.00, T3 3270.00, T4 3280.00).
    assert_eq!(
        movements(&mut tx, &branch, "USD").await,
        vec![(
            "ADJUST".to_owned(),
            d("100"),
            d("3250.00"),
            Some(d("32.50000000"))
        )],
        "T1: ADJUST +100 at tier 1's BUY 32.50"
    );
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("100"), d("3250.00")),
        "T1: position 100 / 3,250.00"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// T2: an approved NEGATIVE_STOCK SELL of 10 USD with no history, publication tier-only: provisional cost 325.00.
#[tokio::test]
async fn t2_short_sale_with_no_history_on_a_tier_only_publication_is_costed_at_tier_1() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(&pool, &branch, 0, &usd_tiers(Some("32.50")), true).await;
    let postings = LedgerCashPostings::new();
    let short_line = seed_sell_line(&pool, &branch, &shift_id, "10", "334.00").await;
    let (approved, short_tx) = approved_negative_stock(&pool, &postings, &branch, short_line).await;

    let mut tx = pool.begin().await.expect("begin");
    let res = postings
        .post_in(
            &mut tx,
            &sell_legs(
                &shift_id,
                short_line,
                "10",
                "334.00",
                exchange_source(&short_tx),
            ),
            Some(approved),
        )
        .await
        .expect("T2: an approved short sale posts on a tier-only publication (main refuses missing_buy_rate)");
    // cost = round2(10 × 32.50) = 325.00; margin = 334.00 − 325.00 = 9.00.
    assert_eq!(
        res.costs,
        vec![LineCost {
            line: short_line,
            cost_basis_thb: money("325.00"),
            realized_margin_thb: Some(delta("9.00")),
        }],
        "T2: provisional cost at tier 1's BUY 32.50"
    );
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("-10"), d("-325.00")),
        "T2: position −10 / −325.00"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// T3: tier 1 has no BUY price (hidden or paused): refused naming USD, and T2–T4's BUY prices are never used.
#[tokio::test]
async fn t3_tier_1_without_a_buy_price_is_refused_and_no_other_tier_is_used() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(&pool, &branch, 0, &usd_tiers(None), true).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    assert_refused_naming(
        &mut tx,
        &postings,
        &[overage(&branch, &shift_id, usd(), "100")],
        None,
        "USD",
        "T3: tier 1 has no BUY",
    )
    .await;
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// T4 (kept): an untiered publication still costs at its `ALL` line.
#[tokio::test]
async fn t4_an_untiered_publication_still_costs_at_its_all_line() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        0,
        &[all_line("USD", "1", Some("32.90"), "33.40")],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    postings
        .post_in(&mut tx, &[overage(&branch, &shift_id, usd(), "100")], None)
        .await
        .expect("an overage posts");
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("100"), d("3290.00")),
        "T4: the ALL line's BUY 32.90"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}
