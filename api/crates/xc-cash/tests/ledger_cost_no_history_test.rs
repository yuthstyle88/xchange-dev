//! Up-front tests for #2794 (child of #2108): stock from a position with no cost history is costed at
//! the branch's current published BUY rate, not at 0 THB.
//!
//! Owner decision D1 (#2108 issuecomment-6227336914) and the answers to Q1–Q3 on #2794:
//! - Q1: the branch's **current** publication (`branches.current_publication_id`), even when it was
//!   published on an earlier day. Only when the branch has no published BUY rate for the currency at
//!   all is the posting refused, with `validationFailed` naming the currency; nothing is written.
//! - Q2: only the line with `denomination_group = 'ALL' AND value_tier = 'ALL'` (the line the quote
//!   reads, `xc-rate/.../published_rates.rs`).
//! - Q3: unit cost = `round8(buy_rate / unit_size)`; cost = `round2(q × unit cost)`.
//!
//! The rule applies only when the position has no history (`last_average` is `None`) in
//! `ledger_postings.rs` `dispose` (zero position) and `adjust_in` (zero position). Positions with
//! history keep today's costs (rows K1, K1b, K2).
//!
//! Every expected figure below was computed with Python's `decimal` (ROUND_HALF_UP), not by hand; the
//! arithmetic is in the comment above each assertion.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Each test gets a fresh schema
//! with every migration of `api/migrations` applied, its own branch, shift and publication ids.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
    let schema = unique("nohist");
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

/// R1: a count overage of +100 USD into a drawer at a branch that never held USD; the current
/// publication's USD ALL/ALL line is BUY 32.50 per unit 1.
#[tokio::test]
async fn r1_overage_with_no_history_is_costed_at_the_current_buy_rate() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        0,
        &[all_line("USD", "1", Some("32.50"), "33.00")],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");

    postings
        .post_in(&mut tx, &[overage(&branch, &shift_id, usd(), "100")], None)
        .await
        .expect("an overage with a published BUY rate posts");

    // unit = round8(32.50 / 1) = 32.50000000; cost = round2(100 × 32.5) = 3250.00;
    // average_after = round8(3250.00 / 100) = 32.50000000.
    assert_eq!(
        movements(&mut tx, &branch, "USD").await,
        vec![(
            "ADJUST".to_owned(),
            d("100"),
            d("3250.00"),
            Some(d("32.50000000"))
        )],
        "R1: ADJUST +100 at the BUY rate 32.50 (main books 0.00)"
    );
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("100"), d("3250.00")),
        "R1: position 100 / 3,250.00"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R2: an approved NEGATIVE_STOCK SELL of 10 USD from a position that was never held, BUY 32.50. The
/// provisional cost is 325.00, and a later BUY of 100 at 34.00 still makes Q-4's correction.
#[tokio::test]
async fn r2_short_sale_with_no_history_is_costed_at_the_current_buy_rate() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        0,
        &[all_line("USD", "1", Some("32.50"), "33.00")],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let short_line = seed_sell_line(&pool, &branch, &shift_id, "10", "330.00").await;
    let (approved, short_tx) = approved_negative_stock(&pool, &postings, &branch, short_line).await;

    let mut tx = pool.begin().await.expect("begin");
    let res = postings
        .post_in(
            &mut tx,
            &sell_legs(
                &shift_id,
                short_line,
                "10",
                "330.00",
                exchange_source(&short_tx),
            ),
            Some(approved),
        )
        .await
        .expect("an approved short sale posts");
    // cost = round2(10 × round8(32.50 / 1)) = 325.00; margin = 330.00 − 325.00 = 5.00.
    assert_eq!(
        res.costs,
        vec![LineCost {
            line: short_line,
            cost_basis_thb: money("325.00"),
            realized_margin_thb: Some(delta("5.00")),
        }],
        "R2: provisional cost 325.00 at the BUY rate (main books 0.00, margin 330.00)"
    );
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("-10"), d("-325.00")),
        "R2: position −10 / −325.00"
    );

    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "3400.00").await;
    postings
        .post_in(
            &mut tx,
            &buy_legs(&shift_id, "100", "3400.00", exchange_source(&unique("buy"))),
            None,
        )
        .await
        .expect("buy 100 at 34.00");
    // Q-4: provisional 325.00, real = round2(10 × 3400.00 / 100) = 340.00, correction = 325.00 − 340.00
    // = −15.00. Position: −325.00 + 3400.00 − 15.00 = 3060.00 over 90; average_after = 34.00000000.
    let adjusts: Vec<_> = movements(&mut tx, &branch, "USD")
        .await
        .into_iter()
        .filter(|m| m.0 == "ADJUST")
        .collect();
    assert_eq!(
        adjusts,
        vec![(
            "ADJUST".to_owned(),
            d("0"),
            d("-15.00"),
            Some(d("34.00000000"))
        )],
        "R2: Q-4's cost-only ADJUST corrects 325.00 to the real 340.00 (main: −340.00 from 0.00)"
    );
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("90"), d("3060.00")),
        "R2: position 90 / 3,060.00 after the buy"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R3a: as R1, but the branch's current publication has no USD line (only EUR).
#[tokio::test]
async fn r3_overage_refused_when_current_publication_has_no_line_for_the_currency() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        0,
        &[all_line("EUR", "1", Some("37.00"), "38.00")],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    assert_refused_naming(
        &mut tx,
        &postings,
        &[overage(&branch, &shift_id, usd(), "100")],
        None,
        "USD",
        "R3a: current publication without a USD line",
    )
    .await;
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R3b: as R1, but the USD ALL/ALL line's `buy_rate` is NULL (sell only).
#[tokio::test]
async fn r3_overage_refused_when_the_buy_rate_is_null() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        0,
        &[all_line("USD", "1", None, "33.00")],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    assert_refused_naming(
        &mut tx,
        &postings,
        &[overage(&branch, &shift_id, usd(), "100")],
        None,
        "USD",
        "R3b: USD line with buy_rate NULL",
    )
    .await;
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R3c (edge case §9, the `dispose` side): an approved short sale from a never-held position at a
/// branch with no published USD BUY rate is refused too, and writes nothing.
#[tokio::test]
async fn r3_short_sale_refused_when_there_is_no_published_buy_rate() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let short_line = seed_sell_line(&pool, &branch, &shift_id, "10", "330.00").await;
    let (approved, short_tx) = approved_negative_stock(&pool, &postings, &branch, short_line).await;
    let mut tx = pool.begin().await.expect("begin");
    assert_refused_naming(
        &mut tx,
        &postings,
        &sell_legs(
            &shift_id,
            short_line,
            "10",
            "330.00",
            exchange_source(&short_tx),
        ),
        Some(approved),
        "USD",
        "R3c: approved short sale with no published BUY rate",
    )
    .await;
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R4: as R1, but the branch has no `current_publication_id` at all.
#[tokio::test]
async fn r4_overage_refused_when_the_branch_has_no_current_publication() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    assert_refused_naming(
        &mut tx,
        &postings,
        &[overage(&branch, &shift_id, usd(), "100")],
        None,
        "USD",
        "R4: no current publication",
    )
    .await;
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R5: a JPY overage of +1,000 with BUY 23.40 per `unit_size` 100.
#[tokio::test]
async fn r5_buy_rate_is_divided_by_the_unit_size() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        0,
        &[all_line("JPY", "100", Some("23.40"), "23.90")],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    postings
        .post_in(
            &mut tx,
            &[overage(&branch, &shift_id, code("JPY"), "1000")],
            None,
        )
        .await
        .expect("a JPY overage posts");
    // unit = round8(23.40 / 100) = 0.23400000; cost = round2(1000 × 0.234) = 234.00
    // (without the division: 23,400.00).
    assert_eq!(
        position(&mut tx, &branch, "JPY").await,
        (d("1000"), d("234.00")),
        "R5: cost = 1000 × 23.40 / 100 = 234.00"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R6 (Q3): the unit cost is rounded to 8 places before it is multiplied. JPY BUY 23.45678950 per 100,
/// overage of +50,000.
#[tokio::test]
async fn r6_unit_cost_is_rounded_to_8_places_before_the_total() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        0,
        &[all_line("JPY", "100", Some("23.45678950"), "23.90")],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    postings
        .post_in(
            &mut tx,
            &[overage(&branch, &shift_id, code("JPY"), "50000")],
            None,
        )
        .await
        .expect("a JPY overage posts");
    // 23.45678950 / 100 = 0.234567895 → round8 = 0.23456790 (half away from zero);
    // 50000 × 0.23456790 = 11728.395 → round2 = 11728.40.
    // Without round8: 50000 × 0.234567895 = 11728.39475 → round2 = 11728.39.
    assert_eq!(
        position(&mut tx, &branch, "JPY").await,
        (d("50000"), d("11728.40")),
        "R6: round8(buy / unit_size) first, then round2 of the total"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R7 (Q1): the current publication counts even when it was published on an earlier day.
#[tokio::test]
async fn r7_a_current_publication_from_an_earlier_day_still_counts() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        3,
        &[all_line("USD", "1", Some("31.75"), "33.00")],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    postings
        .post_in(&mut tx, &[overage(&branch, &shift_id, usd(), "40")], None)
        .await
        .expect("an overage posts against a publication made 3 days ago");
    // cost = round2(40 × round8(31.75 / 1)) = 1270.00.
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("40"), d("1270.00")),
        "R7: the publication of 3 days ago is the current one"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// R8 (Q2): only the `denomination_group = 'ALL' AND value_tier = 'ALL'` line is used when the
/// publication also has other groups and value tiers of the same currency. The other lines are
/// inserted first **and** their names sort before `ALL` (`'100' < 'ALL'`, `'A1' < 'ALL'` in both the C
/// and en_US collations), so a reader that drops either filter picks one of them whether Postgres
/// returns heap order or the order of the `(publication_id, currency_code, denomination_group,
/// value_tier)` unique index.
#[tokio::test]
async fn r8_only_the_all_group_all_tier_line_is_used() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    publish(
        &pool,
        &branch,
        0,
        &[
            (
                "USD",
                "100",
                "A1",
                "1",
                Some("29.00"),
                "33.00",
                None,
                Some("5000.00"),
            ),
            ("USD", "100", "ALL", "1", Some("30.00"), "33.00", None, None),
            (
                "USD",
                "ALL",
                "A1",
                "1",
                Some("31.00"),
                "33.00",
                None,
                Some("10000.00"),
            ),
            (
                "USD",
                "ALL",
                "T2",
                "1",
                Some("31.20"),
                "33.00",
                Some("10000.00"),
                None,
            ),
            all_line("USD", "1", Some("32.50"), "33.00"),
        ],
        true,
    )
    .await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    postings
        .post_in(&mut tx, &[overage(&branch, &shift_id, usd(), "100")], None)
        .await
        .expect("an overage posts");
    // ALL/ALL: round2(100 × 32.50) = 3250.00 (100/A1 2900.00, 100/ALL 3000.00, ALL/A1 3100.00,
    // ALL/T2 3120.00).
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("100"), d("3250.00")),
        "R8: the ALL/ALL line's BUY 32.50, never another group's or tier's"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Kept behaviour: positions with history are costed as today
// ─────────────────────────────────────────────────────────────────────────────

/// Buy 100 at 32.00 and sell all 100: the position is exactly 0 and the last `average_after` is 32.
async fn buy_then_sell_all(
    pool: &PgPool,
    tx: &mut Transaction<'_, sqlx::Postgres>,
    postings: &LedgerCashPostings,
    branch: &BranchId,
    shift_id: &str,
) {
    fund_drawer_thb(tx, postings, branch, shift_id, "3200.00").await;
    postings
        .post_in(
            tx,
            &buy_legs(shift_id, "100", "3200.00", exchange_source(&unique("buy"))),
            None,
        )
        .await
        .expect("buy 100 at 32.00");
    let sell_all = seed_sell_line(pool, branch, shift_id, "100", "3300.00").await;
    postings
        .post_in(
            tx,
            &sell_legs(
                shift_id,
                sell_all,
                "100",
                "3300.00",
                exchange_source(&unique("sell")),
            ),
            None,
        )
        .await
        .expect("sell 100");
    assert_eq!(
        position(tx, branch, "USD").await,
        (d("0"), d("0.00")),
        "position exactly 0 with history"
    );
}

/// K1 (kept, `dispose`): a short sale from an exactly-zero position **with** history is costed at the
/// last `average_after` (32), not at the published BUY rate (40.00).
#[tokio::test]
async fn k1_short_sale_with_history_keeps_the_last_average() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let short_line = seed_sell_line(&pool, &branch, &shift_id, "10", "330.00").await;

    let mut tx = pool.begin().await.expect("begin");
    buy_then_sell_all(&pool, &mut tx, &postings, &branch, &shift_id).await;
    tx.commit().await.expect("commit the history");
    publish(
        &pool,
        &branch,
        0,
        &[all_line("USD", "1", Some("40.00"), "41.00")],
        true,
    )
    .await;
    let (approved, short_tx) = approved_negative_stock(&pool, &postings, &branch, short_line).await;

    let mut tx = pool.begin().await.expect("begin");
    let res = postings
        .post_in(
            &mut tx,
            &sell_legs(
                &shift_id,
                short_line,
                "10",
                "330.00",
                exchange_source(&short_tx),
            ),
            Some(approved),
        )
        .await
        .expect("an approved short sale posts");
    // cost = round2(10 × 32) = 320.00; margin = 330.00 − 320.00 = 10.00 (BUY 40.00 would give 400.00).
    assert_eq!(
        res.costs,
        vec![LineCost {
            line: short_line,
            cost_basis_thb: money("320.00"),
            realized_margin_thb: Some(delta("10.00")),
        }],
        "K1: cost at the last average_after 32, as today"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// K1b (kept, `adjust_in`): an overage into an exactly-zero position **with** history is costed at the
/// last `average_after` (32), not at the published BUY rate (40.00).
#[tokio::test]
async fn k1b_overage_into_zero_position_with_history_keeps_the_last_average() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    buy_then_sell_all(&pool, &mut tx, &postings, &branch, &shift_id).await;
    tx.commit().await.expect("commit the history");
    publish(
        &pool,
        &branch,
        0,
        &[all_line("USD", "1", Some("40.00"), "41.00")],
        true,
    )
    .await;

    let mut tx = pool.begin().await.expect("begin");
    postings
        .post_in(&mut tx, &[overage(&branch, &shift_id, usd(), "5")], None)
        .await
        .expect("an overage posts");
    // cost = round2(5 × 32) = 160.00 (BUY 40.00 would give 200.00).
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("5"), d("160.00")),
        "K1b: cost at the last average_after 32, as today"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// K2 (kept): an overage into a position above zero is costed at its average, and a branch with no
/// publication at all still posts it (the rate is read only when there is no history).
#[tokio::test]
async fn k2_overage_into_a_held_position_needs_no_publication() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "3200.00").await;
    postings
        .post_in(
            &mut tx,
            &buy_legs(&shift_id, "100", "3200.00", exchange_source(&unique("buy"))),
            None,
        )
        .await
        .expect("buy 100 at 32.00");
    postings
        .post_in(&mut tx, &[overage(&branch, &shift_id, usd(), "5")], None)
        .await
        .expect("an overage into a held position posts without any publication");
    // cost = round2(5 × 3200.00 / 100) = 160.00; position 105 / 3360.00.
    assert_eq!(
        position(&mut tx, &branch, "USD").await,
        (d("105"), d("3360.00")),
        "K2: cost at the average 32, as today"
    );
    tx.rollback().await.expect("rollback");
    pool.close().await;
}
