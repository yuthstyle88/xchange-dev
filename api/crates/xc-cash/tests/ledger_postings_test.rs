//! Up-front tests for LG-2 (#924; design `docs/design/blueprint/ledger-inventory.md` §4.1–§4.6, §9, §10.1):
//! the `CashPostings` port final shape and `LedgerCashPostings` implementation.
//! Written by gemini/pool-9 as test-writer per ticket author spec and review F1–F8; revised by
//! claude/14cc24bd for claude/89bd61d9's review R1–R4 (issuecomment-6106358870).
//!
//! §4.5 refuses a posting that takes `DRAWER` or `SAFE` below zero, so every test that pays THB out of
//! a drawer funds it first (`fund_drawer_thb`: a non-cash deposit into `BANK`, `BANK → SAFE`,
//! `SAFE → DRAWER`).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.
//! Each test gets a fresh schema with every migration of `api/migrations` applied.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pos_error::AppError;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Connection, PgPool, Row, Transaction};
use uuid::Uuid;

use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::domain::inventory::repository::InventoryRepository;
use xc_cash::domain::ledger::posting::{PostingId, SourceRef, SourceType};
use xc_cash::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, LineCost, PostingRequest, PostingResult,
    ReversedCost, TransactionLineId,
};
use xc_cash::infrastructure::inventory::repository::PgInventoryRepository;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, MoneyDelta};
use xc_shift::domain::shift::value_objects::balances::Balances;

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

/// A signed THB amount (`realized_margin_thb`, `difference_thb`: `MoneyDelta`, lane controller's ruling
/// issuecomment-6106564990, because a margin or a difference can be negative).
fn delta(s: &str) -> MoneyDelta {
    let minor = (d(s) * Decimal::from(100)).round().to_i128().expect("fits");
    MoneyDelta::from_minor(minor)
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
    let schema = unique("lg2");
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
    sqlx::query("INSERT INTO ops_state (branch_id, phase, safe) VALUES ($1, 'OPEN', '{}'::jsonb)")
        .bind(&branch)
        .execute(&pool)
        .await
        .expect("insert ops_state");

    (pool, BranchId::new(branch))
}

async fn seed_customer(pool: &PgPool, _branch: &BranchId) -> String {
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
    cust_id
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'test')",
    )
    .bind(id)
    .bind(ORG)
    .execute(pool)
    .await
    .expect("insert user");
    id
}

async fn seed_shift(pool: &PgPool, branch: &BranchId) -> String {
    let teller_id = seed_user(pool).await;
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

// Note: xc-exchange is not a dependency of xc-cash (xc-exchange depends on xc-cash,
// preventing a circular dependency). Per review F6, we document here why xc-exchange's
// store cannot be reached from xc-cash tests and SQL is used instead for transaction_lines.
#[allow(clippy::too_many_arguments)]
async fn seed_transaction_line(
    pool: &PgPool,
    branch: &BranchId,
    shift_id: &str,
    shop_side: &str,
    currency: CurrencyCode,
    quantity: Decimal,
    thb_amount: Decimal,
    cost_basis_thb: Option<Decimal>,
    realized_margin_thb: Option<Decimal>,
) -> TransactionLineId {
    let cust_id = seed_customer(pool, branch).await;
    let tx_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    let doc_ref = unique("DOC");

    sqlx::query(
        "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
         customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
         recorded_at, recorded_by, status, completed_at) \
         VALUES ($1, $2::uuid, $3, $4, 'LIVE', $5, $6, 'hash', $7, 'Somchai', 'THA', 'system', 'WALK-IN', \
         '', current_date, now(), now(), 'system', 'COMPLETED', now())",
    )
    .bind(tx_id)
    .bind(ORG)
    .bind(&doc_ref)
    .bind(branch.as_str())
    .bind(shift_id)
    .bind(unique("sub"))
    .bind(cust_id)
    .execute(pool)
    .await
    .expect("insert transaction");

    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
         quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, cost_basis_thb, \
         realized_margin_thb, line_no, created_at, updated_at) \
         VALUES ($1, $2::uuid, $3, $4, $5, $6, 1, 1.0, $7, 0, $7, $8, $9, 1, now(), now())",
    )
    .bind(line_id)
    .bind(ORG)
    .bind(tx_id)
    .bind(shop_side)
    .bind(currency.as_str())
    .bind(quantity)
    .bind(thb_amount)
    .bind(cost_basis_thb)
    .bind(realized_margin_thb)
    .execute(pool)
    .await
    .expect("insert transaction line");

    line_id
}

fn source(id: &str) -> SourceRef {
    SourceRef {
        source_type: SourceType::Exchange,
        source_id: id.to_string(),
        source_key: "complete".to_string(),
    }
}

fn source_reversal(id: &str) -> SourceRef {
    SourceRef {
        source_type: SourceType::Reversal,
        source_id: id.to_string(),
        source_key: "reversal".to_string(),
    }
}

/// Helper asserting LG-1's ledger account entries (satisfies F4 and §6 Check 5).
async fn assert_posting_entries(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    posting_id: PostingId,
    expected: &[(&str, CurrencyCode, Decimal, Decimal)], // (kind, currency, quantity_signed, valuation_thb)
) {
    let rows: Vec<(String, String, Decimal, Decimal)> = sqlx::query_as(
        "SELECT a.kind, e.currency, e.quantity_signed, e.valuation_thb \
         FROM ledger_entries e \
         JOIN cash_accounts a ON a.id = e.account_id \
         WHERE e.posting_id = $1 \
         ORDER BY a.kind, e.currency, e.quantity_signed",
    )
    .bind(posting_id.0)
    .fetch_all(&mut **tx)
    .await
    .expect("fetch ledger entries");

    assert_eq!(
        rows.len(),
        expected.len(),
        "expected {} ledger entries, found {}",
        expected.len(),
        rows.len()
    );

    for (kind, cur, qty, val) in expected {
        let found = rows.iter().any(|(r_kind, r_cur, r_qty, r_val)| {
            r_kind == kind && r_cur.trim() == cur.as_str() && *r_qty == *qty && *r_val == *val
        });
        assert!(
            found,
            "missing expected entry: kind={kind}, cur={cur}, qty={qty}, val={val}; actual entries: {rows:?}"
        );
    }

    let sum_val: Decimal = rows.iter().map(|(_, _, _, v)| *v).sum();
    assert_eq!(
        sum_val,
        Decimal::ZERO,
        "§6 Check 5: posting valuations must sum to 0"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 / §10.1 rows (1–11) and the negative-stock refusals (12–14, review R4)
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// Funding (§4.5) and the parity replay's mapping (R2)
// ─────────────────────────────────────────────────────────────────────────────

/// §4.5 refuses a posting that takes `DRAWER` or `SAFE` below zero, so a test that pays THB out of a
/// fresh branch funds it first with one posting: a non-cash THB deposit into `BANK` and `BANK → SAFE`
/// (§4.4 EXTERNAL in). The safe names the branch; `BANK` nets to 0, so a later assertion on `BANK` sees
/// only what the test itself sent there.
async fn fund_safe_thb(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    postings: &LedgerCashPostings,
    branch: &BranchId,
    amount: &str,
) {
    let src = SourceRef {
        source_type: SourceType::Transfer,
        source_id: unique("fund-safe"),
        source_key: "external-in".to_string(),
    };
    let legs = [
        PostingRequest {
            from: CashPlace::Customer,
            to: CashPlace::Bank,
            currency: thb(),
            quantity: fa(amount),
            source: src.clone(),
            line: None,
            reverses: None,
        },
        PostingRequest {
            from: CashPlace::Bank,
            to: CashPlace::Safe(branch.clone()),
            currency: thb(),
            quantity: fa(amount),
            source: src,
            line: None,
            reverses: None,
        },
    ];
    postings
        .post_in(tx, &legs, None)
        .await
        .expect("fund: a THB deposit into BANK and BANK -> SAFE (EXTERNAL in)");
}

/// `fund_safe_thb`, then `SAFE → DRAWER` (§4.4 OPENING), so the drawer can pay `amount` THB.
async fn fund_drawer_thb(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    postings: &LedgerCashPostings,
    branch: &BranchId,
    shift_id: &str,
    amount: &str,
) {
    fund_safe_thb(tx, postings, branch, amount).await;
    let open = PostingRequest {
        from: CashPlace::Safe(branch.clone()),
        to: CashPlace::Drawer(shift_id.to_string()),
        currency: thb(),
        quantity: fa(amount),
        source: SourceRef {
            source_type: SourceType::Opening,
            source_id: unique("fund-open"),
            source_key: "opening".to_string(),
        },
        line: None,
        reverses: None,
    };
    postings
        .post_in(tx, &[open], None)
        .await
        .expect("fund: SAFE -> DRAWER (OPENING)");
}

/// `Money` carries at most `Money::MAX_INPUT_UNITS` (100,000,000.00), and `expected_in` answers `Money`, so it
/// cannot carry a balance above it. Amounts above it are left out
/// of the comparison on every side, and the cases where that happens are listed in the skip table.
const CAP_MINOR: i64 = 10_000_000_000;

/// Foreign currencies the parity replay funds the branch with.
const FUNDED: [&str; 7] = ["USD", "EUR", "GBP", "JPY", "CNY", "ZWG", "AED"];

fn jdec(v: &Value) -> Decimal {
    match v {
        Value::String(s) => d(s),
        Value::Number(n) => d(&n.to_string()),
        other => panic!("not an amount: {other}"),
    }
}

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).unwrap_or_else(|_| panic!("a currency code: {s}"))
}

fn minor_of(v: Decimal) -> i64 {
    (v * Decimal::from(100))
        .round()
        .to_i64()
        .expect("fits in i64")
}

/// A balance set as `(code, hundredths)`, zero and over-cap entries left out, sorted by code.
/// The `bool` is true when an over-cap entry was left out.
fn comparable(pairs: impl IntoIterator<Item = (String, i64)>) -> (Vec<(String, i64)>, bool) {
    let mut dropped = false;
    let mut out: Vec<(String, i64)> = pairs
        .into_iter()
        .filter(|(_, m)| *m != 0)
        .filter(|(_, m)| {
            let keep = m.abs() <= CAP_MINOR;
            dropped |= !keep;
            keep
        })
        .collect();
    out.sort();
    (out, dropped)
}

fn from_balances(b: &Balances) -> Vec<(String, i64)> {
    comparable(
        b.non_zero()
            .into_iter()
            .map(|(c, m)| (c.as_str().to_string(), m.minor())),
    )
    .0
}

fn from_fixture(map: &Value) -> (Vec<(String, i64)>, bool) {
    comparable(
        map.as_object()
            .expect("an amounts object")
            .iter()
            .map(|(k, v)| (k.clone(), minor_of(jdec(v)))),
    )
}

/// v2's THB effect on the place the payment method names: `round_half_up(amount × rate, 2)`, minus the
/// fee on a BUY and plus it on a SELL. Pinned against every `v2_stock_cases.json` delta before use.
fn v2_thb(t: &Value) -> Decimal {
    let principal = (jdec(&t["amount"]) * jdec(&t["rate"]))
        .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    let fee = jdec(&t["fee"]);
    if t["type"] == "BUY" {
        fee - principal
    } else {
        principal + fee
    }
}

/// Test-author choice 6 (accepted, issuecomment-6097791219): `tx` = the two legs, the foreign one with a
/// line, the THB one in `Drawer` for Cash and in `Bank` otherwise. A zero THB leg is not sent
/// (`ledger_entries.quantity_signed <> 0`).
fn exchange_legs(shift_id: &str, t: &Value, source_id: &str) -> Vec<PostingRequest> {
    let src = source(source_id);
    let foreign = code(t["currency"].as_str().expect("currency"));
    let drawer = CashPlace::Drawer(shift_id.to_string());
    let (from, to) = if t["type"] == "BUY" {
        (CashPlace::Customer, drawer.clone())
    } else {
        (drawer.clone(), CashPlace::Customer)
    };
    let mut legs = vec![PostingRequest {
        from,
        to,
        currency: foreign,
        quantity: ForeignAmount::from_decimal(jdec(&t["amount"])),
        source: src.clone(),
        line: Some(Uuid::new_v4()),
        reverses: None,
    }];
    let thb_amount = v2_thb(t);
    let place = if t["method"] == "Cash" {
        drawer
    } else {
        CashPlace::Bank
    };
    if !thb_amount.is_zero() {
        let (from, to) = if thb_amount > Decimal::ZERO {
            (CashPlace::Customer, place)
        } else {
            (place, CashPlace::Customer)
        };
        legs.push(PostingRequest {
            from,
            to,
            currency: thb(),
            quantity: ForeignAmount::from_decimal(thb_amount.abs()),
            source: src,
            line: None,
            reverses: None,
        });
    }
    legs
}

/// `open` = OPENING `SAFE → DRAWER` per currency; `move` = `SAFE ↔ DRAWER`.
fn place_move(
    branch: &BranchId,
    shift_id: &str,
    currency: CurrencyCode,
    amount: Decimal,
    into_drawer: bool,
    source: SourceRef,
) -> PostingRequest {
    let (safe, drawer) = (
        CashPlace::Safe(branch.clone()),
        CashPlace::Drawer(shift_id.to_string()),
    );
    let (from, to) = if into_drawer {
        (safe, drawer)
    } else {
        (drawer, safe)
    };
    PostingRequest {
        from,
        to,
        currency,
        quantity: ForeignAmount::from_decimal(amount),
        source,
        line: None,
        reverses: None,
    }
}

fn opening_legs(
    branch: &BranchId,
    shift_id: &str,
    opening: &Value,
    id: &str,
) -> Vec<PostingRequest> {
    opening
        .as_object()
        .expect("an opening object")
        .iter()
        .map(|(c, v)| (c, jdec(v)))
        .filter(|(_, v)| !v.is_zero())
        .map(|(c, v)| {
            place_move(
                branch,
                shift_id,
                code(c),
                v,
                true,
                SourceRef {
                    source_type: SourceType::Opening,
                    source_id: id.to_string(),
                    source_key: "opening".to_string(),
                },
            )
        })
        .collect()
}

/// One step through the port. LG-4 (#929, lane controller's 1:1 ruling): `InterimCashPostings` is removed, so
/// only its side of this replay goes; `LedgerCashPostings` still gets the posting whole.
async fn post_both(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    ledger: &LedgerCashPostings,
    reqs: &[PostingRequest],
    what: &str,
) {
    if reqs.is_empty() {
        return;
    }
    ledger
        .post_in(tx, reqs, None)
        .await
        .unwrap_or_else(|e| panic!("{what}: LedgerCashPostings refused: {e:?}"));
}

/// Where v2 refused for stock, `LedgerCashPostings` refuses `stockInsufficient` (§4.5). Run in a
/// savepoint so the replay goes on.
async fn ledger_refuses_stock(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    ledger: &LedgerCashPostings,
    reqs: &[PostingRequest],
    what: &str,
) {
    let mut sp = Connection::begin(&mut **tx).await.expect("savepoint");
    match ledger.post_in(&mut sp, reqs, None).await {
        Err(AppError::Domain(msg)) if msg.contains("stockInsufficient") => {}
        other => panic!(
            "{what}: v2 refused for stock, so the ledger must refuse stockInsufficient; got {other:?}"
        ),
    }
    sp.rollback().await.expect("rollback savepoint");
}

/// After a step: the drawer through the port equals the case's `expected` (LG-4: the interim side is removed).
async fn assert_drawer(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    ledger: &LedgerCashPostings,
    shift_id: &str,
    expected: &Value,
    what: &str,
) -> bool {
    let place = CashPlace::Drawer(shift_id.to_string());
    let l = ledger
        .expected_in(tx, &place)
        .await
        .unwrap_or_else(|e| panic!("{what}: ledger expected_in: {e:?}"));
    let (want, dropped) = from_fixture(expected);
    assert_eq!(
        from_balances(&l),
        want,
        "{what}: the drawer differs from the fixture's expected"
    );
    dropped
}

/// A branch has one OPEN shift at a time (`shifts_one_open_per_branch_v2`), as in v2: the replay closes
/// the branch's open shift before it opens the next. Postings to a closed shift's drawer are not made.
async fn next_shift(pool: &PgPool, branch: &BranchId) -> String {
    sqlx::query("UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'")
        .bind(branch.as_str())
        .execute(pool)
        .await
        .expect("close the open shift");
    seed_shift(pool, branch).await
}

/// The parity replay funds one branch once: THB through `BANK → SAFE`, each foreign currency by a
/// non-cash BUY into a funding drawer (so the position holds it, §4.3) moved on to the safe (§4.4).
async fn fund_parity_branch(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    pool: &PgPool,
    ledger: &LedgerCashPostings,
    branch: &BranchId,
) {
    fund_safe_thb(tx, ledger, branch, "1000000000000").await;
    let fshift = next_shift(pool, branch).await;
    for c in FUNDED {
        let id = format!("fund-buy-{c}");
        let buy = [
            PostingRequest {
                from: CashPlace::Customer,
                to: CashPlace::Drawer(fshift.clone()),
                currency: code(c),
                quantity: fa("10000000000"),
                source: source(&id),
                line: Some(Uuid::new_v4()),
                reverses: None,
            },
            PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Customer,
                currency: thb(),
                quantity: fa("10000000000"),
                source: source(&id),
                line: None,
                reverses: None,
            },
        ];
        ledger
            .post_in(tx, &buy, None)
            .await
            .unwrap_or_else(|e| panic!("fund: BUY {c}: {e:?}"));
        let mv = place_move(
            branch,
            &fshift,
            code(c),
            d("10000000000"),
            false,
            SourceRef {
                source_type: SourceType::Transfer,
                source_id: format!("fund-move-{c}"),
                source_key: "to-safe".to_string(),
            },
        );
        ledger
            .post_in(tx, &[mv], None)
            .await
            .unwrap_or_else(|e| panic!("fund: move {c} to the safe: {e:?}"));
    }
}

/// A stock scenario's transaction or movement: its requests, its effect on the drawer, its label.
type StockOp = (Vec<PostingRequest>, Vec<(String, Decimal)>, String);

fn read_fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../xc-shift/tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed reading {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} parses: {e}"))
}

/// The cases the replay does not map, with the reason (R2: none dropped silently).
const SKIPPED_STOCK_SCENARIOS: usize = 25; // §4.5: v2 lets the drawer go below zero (`firstNegative`); the ledger refuses
const SHIFT_POSTED: usize = 218; // open (new shift), move, tx accepted by v2
const SHIFT_STOCK_REFUSED: usize = 75; // v2 "… ไม่เพียงพอ …": the ledger refuses stockInsufficient too
const SHIFT_ONLY_SKIPPED: usize = 90; // v2 refusals about the shift or the request, not stock: nothing to post
const SHIFT_REPLAYS: usize = 8; // v2 replays a request it already applied: nothing new to post
const SHIFT_NO_POSTING: usize = 49; // close and abandon accepted by v2: post nothing
const CAPPED_COMPARISONS: usize = 19; // 4 shift steps, 12 SELL deltas, 3 stock scenarios: an amount above `CAP_MINOR`
const SKIPPED_DELTA_REFUSALS: usize = 4; // not a stock currency code (`NOT_A_STOCK_CURRENCY`: THB, XXX, usd, ""): no posting exists to send

/// Row 1: v2 parity row (§10.1; R2). Every case of `v2_shift_cases.json` (30 scenarios) and
/// `v2_stock_cases.json` (604 deltas, 40 scenarios) gives the same drawer through
/// `LedgerCashPostings`, equal to the fixture (LG-4: the `InterimCashPostings` side is removed).
///
/// Mapping (test-author choice 6, accepted in issuecomment-6097791219): `open` = OPENING SAFE→DRAWER;
/// `move` = SAFE↔DRAWER; `tx` = the two legs (Cash THB in `Drawer`, other methods in `Bank`);
/// `count`/`close` (and `abandon`, a screen's request resolution) post nothing; v2's shift-only refusals
/// are skipped. v2's stock refusals
/// (`… ไม่เพียงพอ …`) are replayed against the ledger, which must refuse `stockInsufficient`.
#[tokio::test]
async fn v2_parity_shift_and_stock_cases() {
    let shift_json = read_fixture("v2_shift_cases.json");
    let stock_json = read_fixture("v2_stock_cases.json");

    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    fund_parity_branch(&mut tx, &pool, &ledger, &branch).await;

    let mut skip_table: Vec<String> = Vec::new();

    // ── v2_shift_cases.json: 30 scenarios, step by step ──
    let scenarios = shift_json["scenarios"].as_array().expect("scenarios");
    assert_eq!(scenarios.len(), 30, "v2_shift_cases.json has 30 scenarios");
    let (mut posted, mut stock_refused, mut shift_only, mut replays, mut no_posting) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    for (i, sc) in scenarios.iter().enumerate() {
        let name = sc["name"].as_str().unwrap_or("");
        let mut shifts: Vec<String> = Vec::new();
        let mut seen_moves: HashSet<String> = HashSet::new();
        for (j, st) in sc["steps"].as_array().expect("steps").iter().enumerate() {
            let what = format!(
                "v2_shift_cases scenario {i} '{name}' step {j} ({})",
                st["do"].as_str().unwrap_or("?")
            );
            let after = st["after"]["shifts"].as_array().expect("after.shifts");
            let open_shift = after
                .iter()
                .rposition(|s| s["status"] == "OPEN")
                .map(|k| shifts.get(k).cloned().unwrap_or_default());
            let id = format!("s{i}-{j}");
            let shift_of = |payload: &Value| -> Option<String> {
                payload["shiftId"]
                    .as_str()
                    .and_then(|s| s.strip_prefix("$SHIFT:"))
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| shifts.get(n).cloned())
            };
            let move_req = |payload: &Value, shift: &str| {
                place_move(
                    &branch,
                    shift,
                    code(payload["currency"].as_str().expect("currency")),
                    jdec(&payload["amount"]),
                    payload["direction"] == "IN",
                    SourceRef {
                        source_type: SourceType::Transfer,
                        source_id: id.clone(),
                        source_key: "move".to_string(),
                    },
                )
            };
            match (st["do"].as_str().unwrap_or(""), st["v2Error"].as_str()) {
                (kind, Some(e)) if e.contains("ไม่เพียงพอ") => {
                    let reqs = match kind {
                        "move" => {
                            let s = shift_of(&st["payload"]).unwrap_or_else(|| {
                                panic!("{what}: a stock refusal names a known shift")
                            });
                            vec![move_req(&st["payload"], &s)]
                        }
                        "tx" => exchange_legs(
                            &open_shift
                                .clone()
                                .unwrap_or_else(|| panic!("{what}: an open shift")),
                            &st["tx"],
                            &id,
                        ),
                        other => panic!("{what}: a stock refusal on `{other}`"),
                    };
                    ledger_refuses_stock(&mut tx, &ledger, &reqs, &what).await;
                    stock_refused += 1;
                }
                (_, Some(_)) => shift_only += 1,
                ("open", None) => {
                    if after.len() > shifts.len() {
                        let s = next_shift(&pool, &branch).await;
                        let reqs = opening_legs(&branch, &s, &st["payload"]["opening"], &id);
                        shifts.push(s);
                        post_both(&mut tx, &ledger, &reqs, &what).await;
                        posted += 1;
                    } else {
                        replays += 1;
                    }
                }
                ("move", None) => {
                    let rid = st["payload"]["requestId"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    if seen_moves.insert(rid) {
                        let s = shift_of(&st["payload"]).unwrap_or_else(|| {
                            panic!("{what}: an accepted move names a known shift")
                        });
                        post_both(&mut tx, &ledger, &[move_req(&st["payload"], &s)], &what).await;
                        posted += 1;
                    } else {
                        replays += 1;
                    }
                }
                ("tx", None) => {
                    let s = open_shift
                        .unwrap_or_else(|| panic!("{what}: an accepted tx has an open shift"));
                    post_both(&mut tx, &ledger, &exchange_legs(&s, &st["tx"], &id), &what).await;
                    posted += 1;
                }
                // `abandon` resolves a request the screen gave up on: shift-only, nothing to post.
                ("count" | "close" | "abandon", None) => no_posting += 1,
                (other, None) => panic!("{what}: unmapped step `{other}`"),
            }
            assert_eq!(
                after.len(),
                shifts.len(),
                "{what}: the replay opened as many shifts as v2"
            );
            for (k, s) in shifts.iter().enumerate() {
                let dropped = assert_drawer(
                    &mut tx,
                    &ledger,
                    s,
                    &after[k]["expected"],
                    &format!("{what}, shift {k}"),
                )
                .await;
                if dropped {
                    skip_table.push(format!(
                        "{what}, shift {k}: an amount above 100,000,000 left out of the comparison"
                    ));
                }
            }
        }
    }
    assert_eq!(
        (posted, stock_refused, shift_only, replays, no_posting),
        (
            SHIFT_POSTED,
            SHIFT_STOCK_REFUSED,
            SHIFT_ONLY_SKIPPED,
            SHIFT_REPLAYS,
            SHIFT_NO_POSTING
        ),
        "v2_shift_cases: every step is accounted for (posted, stock refusals, shift-only refusals skipped, replays, close/abandon)"
    );

    // ── v2_stock_cases.json: 604 deltas, each from a fresh drawer ──
    let deltas = stock_json["deltas"].as_array().expect("deltas");
    assert_eq!(deltas.len(), 604, "v2_stock_cases.json has 604 deltas");
    let mut deltas_replayed = 0usize;
    for (n, dl) in deltas.iter().enumerate() {
        let what = format!(
            "v2_stock_cases delta {n} ({} {} {} @ {} fee {} {})",
            dl["type"], dl["currency"], dl["amount"], dl["rate"], dl["fee"], dl["method"]
        );
        let foreign = dl["currency"].as_str().expect("currency");
        let v2_thb_delta = dl["v2"].get("THB").map(jdec).unwrap_or(Decimal::ZERO);
        let cash = dl["method"] == "Cash";
        assert_eq!(
            if cash { v2_thb(dl) } else { Decimal::ZERO },
            v2_thb_delta,
            "{what}: the replay's THB amount is v2's"
        );
        // Open the drawer with exactly what the delta pays out.
        let mut opening = serde_json::Map::new();
        if dl["type"] == "SELL" {
            opening.insert(foreign.to_string(), dl["amount"].clone());
        } else if v2_thb_delta < Decimal::ZERO {
            opening.insert(
                "THB".to_string(),
                Value::String((-v2_thb_delta).to_string()),
            );
        }
        let opening = Value::Object(opening);
        let s = next_shift(&pool, &branch).await;
        let id = format!("d{n}");
        let mut reqs = opening_legs(&branch, &s, &opening, &format!("{id}-open"));
        post_both(&mut tx, &ledger, &reqs, &what).await;
        reqs = exchange_legs(&s, dl, &id);
        post_both(&mut tx, &ledger, &reqs, &what).await;
        let mut expected: HashMap<String, Decimal> = HashMap::new();
        for (c, v) in opening
            .as_object()
            .expect("opening")
            .iter()
            .chain(dl["v2"].as_object().expect("v2"))
        {
            *expected.entry(c.clone()).or_insert(Decimal::ZERO) += jdec(v);
        }
        let expected = Value::Object(
            expected
                .into_iter()
                .map(|(c, v)| (c, Value::String(v.to_string())))
                .collect(),
        );
        if assert_drawer(&mut tx, &ledger, &s, &expected, &what).await {
            skip_table.push(format!(
                "{what}: an amount above 100,000,000 left out of the comparison"
            ));
        }
        deltas_replayed += 1;
    }
    for r in stock_json["deltaRefusals"]
        .as_array()
        .expect("deltaRefusals")
    {
        skip_table.push(format!(
            "v2_stock_cases deltaRefusal {}: not a stock currency code ({}); no posting exists to send",
            r["currency"], r["error"]
        ));
    }

    // ── v2_stock_cases.json: 40 scenarios, each from a fresh drawer ──
    let stock_scenarios = stock_json["scenarios"].as_array().expect("scenarios");
    assert_eq!(
        stock_scenarios.len(),
        40,
        "v2_stock_cases.json has 40 scenarios"
    );
    let (mut scen_replayed, mut scen_skipped) = (0usize, 0usize);
    for (n, sc) in stock_scenarios.iter().enumerate() {
        let what = format!("v2_stock_cases scenario {n}");
        if !sc["firstNegative"].is_null() {
            skip_table.push(format!(
                "{what}: v2 takes {} below zero; §4.5 refuses that without an approved NEGATIVE_STOCK",
                sc["firstNegative"]
            ));
            scen_skipped += 1;
            continue;
        }
        let s = next_shift(&pool, &branch).await;
        let mut balance: HashMap<String, Decimal> = HashMap::new();
        for (c, v) in sc["opening"].as_object().expect("opening") {
            *balance.entry(c.clone()).or_insert(Decimal::ZERO) += jdec(v);
        }
        let open = opening_legs(&branch, &s, &sc["opening"], &format!("ss{n}-open"));
        post_both(&mut tx, &ledger, &open, &what).await;
        // Each transaction and movement, with its effect on the drawer.
        let mut ops: Vec<StockOp> = Vec::new();
        for (k, t) in sc["transactions"]
            .as_array()
            .expect("transactions")
            .iter()
            .enumerate()
        {
            let q = jdec(&t["amount"]);
            let mut effect = vec![(
                t["currency"].as_str().expect("currency").to_string(),
                if t["type"] == "BUY" { q } else { -q },
            )];
            if t["method"] == "Cash" {
                effect.push(("THB".to_string(), v2_thb(t)));
            }
            ops.push((
                exchange_legs(&s, t, &format!("ss{n}-t{k}")),
                effect,
                format!("transaction {k}"),
            ));
        }
        for (k, mv) in sc["movements"]
            .as_array()
            .expect("movements")
            .iter()
            .enumerate()
        {
            let into = mv["direction"] == "IN";
            let q = jdec(&mv["amount"]);
            let c = mv["currency"].as_str().expect("currency");
            let req = place_move(
                &branch,
                &s,
                code(c),
                q,
                into,
                SourceRef {
                    source_type: SourceType::Transfer,
                    source_id: format!("ss{n}-m{k}"),
                    source_key: "move".to_string(),
                },
            );
            ops.push((
                vec![req],
                vec![(c.to_string(), if into { q } else { -q })],
                format!("movement {k}"),
            ));
        }
        // v2 gives only the end state, so any order reaches it; take one that never goes below zero.
        while !ops.is_empty() {
            let pick = ops
                .iter()
                .position(|(_, effect, _)| {
                    effect.iter().all(|(c, v)| {
                        balance.get(c).copied().unwrap_or(Decimal::ZERO) + *v >= Decimal::ZERO
                    })
                })
                .unwrap_or_else(|| panic!("{what}: no order keeps the drawer at or above zero"));
            let (reqs, effect, label) = ops.remove(pick);
            for (c, v) in effect {
                *balance.entry(c).or_insert(Decimal::ZERO) += v;
            }
            post_both(&mut tx, &ledger, &reqs, &format!("{what} {label}")).await;
        }
        if assert_drawer(&mut tx, &ledger, &s, &sc["v2"], &what).await {
            skip_table.push(format!(
                "{what}: an amount above 100,000,000 left out of the comparison"
            ));
        }
        scen_replayed += 1;
    }

    eprintln!("v2 parity skip table ({} entries):", skip_table.len());
    for row in &skip_table {
        eprintln!("  - {row}");
    }
    assert_eq!(
        (deltas_replayed, scen_replayed, scen_skipped),
        (604, 40 - SKIPPED_STOCK_SCENARIOS, SKIPPED_STOCK_SCENARIOS),
        "v2_stock_cases: every delta replayed, every scenario replayed or in the skip table"
    );
    assert_eq!(
        skip_table.len(),
        SKIPPED_STOCK_SCENARIOS + SKIPPED_DELTA_REFUSALS + CAPPED_COMPARISONS,
        "the skip table lists every case not compared in full"
    );

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// Row 2: A BUY posts a lot with a NULL margin (§4.3, §9).
/// Asserts return values, inventory lot & position, and LG-1 account entries (F4).
#[tokio::test]
async fn buy_posts_a_lot_and_null_margin() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let line_id = Uuid::new_v4();

    let postings = LedgerCashPostings::new();
    let inventory = PgInventoryRepository::new(pool.clone());

    let req_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("buy-1"),
        line: Some(line_id),
        reverses: None,
    };
    let req_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3200.00"),
        source: source("buy-1"),
        line: None,
        reverses: None,
    };

    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "3200.00").await;
    let res: PostingResult = postings
        .post_in(&mut tx, &[req_usd, req_thb], None)
        .await
        .expect("buy posting");

    assert_eq!(res.costs.len(), 1, "one LineCost for the line");
    let cost: &LineCost = &res.costs[0];
    assert_eq!(cost.line, line_id);
    assert_eq!(
        res.costs[0].cost_basis_thb,
        money("3200.00"),
        "a BUY freezes cost basis at THB paid"
    );
    assert_eq!(
        res.costs[0].realized_margin_thb, None,
        "a BUY recognizes no margin"
    );
    assert!(res.reversed.is_empty(), "a BUY produces no ReversedCost");

    // Inventory position
    let pos = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("inventory position");
    assert_eq!(pos.quantity, d("100"), "inventory position quantity");
    assert_eq!(pos.cost_thb, d("3200.00"), "inventory position cost_thb");

    // LG-1 account entries (F4)
    assert_posting_entries(
        &mut tx,
        res.posting_id,
        &[
            ("CUSTOMER", usd(), d("-100"), d("-3200.00")),
            ("DRAWER", usd(), d("100"), d("3200.00")),
            ("CUSTOMER", thb(), d("3200.00"), d("3200.00")),
            ("DRAWER", thb(), d("-3200.00"), d("-3200.00")),
        ],
    )
    .await;

    tx.commit().await.expect("commit");
    pool.close().await;
}

/// Row 3: A SELL freezes cost and margin (§4.3, §9).
/// Asserts return values, inventory position, transaction_lines columns, and LG-1 entries (F4).
#[tokio::test]
async fn sell_freezes_cost_and_margin() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();
    let inventory = PgInventoryRepository::new(pool.clone());

    // Initial position: 100 USD acquired at 32.00 (cost 3,200.00)
    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "3200.00").await;
    let buy_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("seed-buy"),
        line: Some(Uuid::new_v4()),
        reverses: None,
    };
    let buy_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3200.00"),
        source: source("seed-buy"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[buy_usd, buy_thb], None)
        .await
        .expect("seed buy");

    // SELL 40 USD for 1,400.00 THB. Allocated cost = 40 * 32.00 = 1,280.00. Margin = 1,400 - 1,280 = 120.00.
    let sell_line_id = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("40"),
        d("1400.00"),
        None,
        None,
    )
    .await;

    let sell_usd = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: usd(),
        quantity: fa("40"),
        source: source("sell-1"),
        line: Some(sell_line_id),
        reverses: None,
    };
    let sell_thb = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: thb(),
        quantity: fa("1400.00"),
        source: source("sell-1"),
        line: None,
        reverses: None,
    };

    let res = postings
        .post_in(&mut tx, &[sell_usd, sell_thb], None)
        .await
        .expect("sell posting");

    assert_eq!(res.costs.len(), 1);
    assert_eq!(res.costs[0].line, sell_line_id);
    assert_eq!(
        res.costs[0].cost_basis_thb,
        money("1280.00"),
        "a SELL freezes allocated cost basis"
    );
    assert_eq!(
        res.costs[0].realized_margin_thb,
        Some(delta("120.00")),
        "realized_margin_thb = thb_amount - cost"
    );

    // Assert cost and margin are frozen in transaction_lines DB table (kills Mutant 1)
    let line_row: (Decimal, Option<Decimal>) = sqlx::query_as(
        "SELECT cost_basis_thb, realized_margin_thb FROM transaction_lines WHERE id = $1",
    )
    .bind(sell_line_id)
    .fetch_one(&mut *tx)
    .await
    .expect("fetch transaction line");
    assert_eq!(
        line_row.0,
        d("1280.00"),
        "cost_basis_thb is frozen on the line"
    );
    assert_eq!(
        line_row.1,
        Some(d("120.00")),
        "realized_margin_thb is frozen on the line"
    );

    let pos = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("inventory position");
    assert_eq!(pos.quantity, d("60"));
    assert_eq!(pos.cost_thb, d("1920.00"));

    // LG-1 entries (F4)
    assert_posting_entries(
        &mut tx,
        res.posting_id,
        &[
            ("CUSTOMER", usd(), d("40"), d("1280.00")),
            ("DRAWER", usd(), d("-40"), d("-1280.00")),
            ("CUSTOMER", thb(), d("-1400.00"), d("-1400.00")),
            ("DRAWER", thb(), d("1400.00"), d("1400.00")),
        ],
    )
    .await;

    tx.commit().await.expect("commit");
    pool.close().await;
}

/// Row 3b (lane controller's ruling, issuecomment-6106564990): a SELL below cost gives a negative
/// `realized_margin_thb`. Buy 100 at 32.00; sell 10 for 300.00 ⇒ cost 320.00, margin −20.00, returned as
/// a `MoneyDelta` and frozen on the line.
#[tokio::test]
async fn sell_below_cost_gives_a_negative_margin() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();

    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "3200.00").await;
    postings
        .post_in(
            &mut tx,
            &buy_legs(
                &shift_id,
                Uuid::new_v4(),
                "100",
                "3200.00",
                source("below-cost-buy"),
            ),
            None,
        )
        .await
        .expect("buy 100 at 32.00");

    let line = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("10"),
        d("300.00"),
        None,
        None,
    )
    .await;
    let res = postings
        .post_in(
            &mut tx,
            &sell_legs(&shift_id, line, "10", "300.00", source("below-cost-sell")),
            None,
        )
        .await
        .expect("sell 10 below cost");
    assert_eq!(
        res.costs,
        vec![LineCost {
            line,
            cost_basis_thb: money("320.00"),
            realized_margin_thb: Some(delta("-20.00")),
        }],
        "a SELL below cost: margin = 300.00 - 320.00 = -20.00"
    );
    let frozen: (Decimal, Option<Decimal>) = sqlx::query_as(
        "SELECT cost_basis_thb, realized_margin_thb FROM transaction_lines WHERE id = $1",
    )
    .bind(line)
    .fetch_one(&mut *tx)
    .await
    .expect("fetch the line");
    assert_eq!(
        frozen,
        (d("320.00"), Some(d("-20.00"))),
        "the negative margin is frozen on the line"
    );

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// Row 4: Non-cash THB goes to BANK, not the drawer (§4.1, §4.3, satisfies F1 and F3).
/// Seeds a BUY first so that stock exists (F1), then asserts the THB leg into Bank positively (F3).
#[tokio::test]
async fn non_cash_thb_goes_to_bank_not_drawer() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();

    let mut tx = pool.begin().await.expect("begin");
    fund_safe_thb(&mut tx, &postings, &branch, "3200.00").await;

    // Open drawer with 3,200.00 THB from Safe so drawer has funds for the buy
    let open_thb = PostingRequest {
        from: CashPlace::Safe(branch.clone()),
        to: CashPlace::Drawer(shift_id.clone()),
        currency: thb(),
        quantity: fa("3200.00"),
        source: SourceRef {
            source_type: SourceType::Opening,
            source_id: "open-seed".to_string(),
            source_key: "opening".to_string(),
        },
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[open_thb], None)
        .await
        .expect("open drawer");

    // F1 fix: Seed a BUY first so that branch holds 100 USD in drawer and position
    let seed_buy_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("seed-buy-f1"),
        line: Some(Uuid::new_v4()),
        reverses: None,
    };
    let seed_buy_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3200.00"),
        source: source("seed-buy-f1"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[seed_buy_usd, seed_buy_thb], None)
        .await
        .expect("seed buy for non_cash test");

    // SELL 50 USD with non-cash payment (Bank Transfer): THB leg moves between Customer and Bank
    let sell_line_id = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("50"),
        d("1600.00"),
        None,
        None,
    )
    .await;
    let req_usd = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: usd(),
        quantity: fa("50"),
        source: source("non-cash-sell"),
        line: Some(sell_line_id),
        reverses: None,
    };
    let req_thb = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Bank,
        currency: thb(),
        quantity: fa("1600.00"),
        source: source("non-cash-sell"),
        line: None,
        reverses: None,
    };

    let res: PostingResult = postings
        .post_in(&mut tx, &[req_usd, req_thb], None)
        .await
        .expect("non cash posting");
    assert_ne!(res.posting_id, PostingId(Uuid::nil()));

    // F3 fix: Positively assert that Bank received the THB 1,600.00
    let bank_balances = postings
        .expected_in(&mut tx, &CashPlace::Bank)
        .await
        .expect("bank expected");
    assert_eq!(
        bank_balances.get(thb()),
        money("1600.00"),
        "positive assertion: Bank received THB 1,600.00"
    );

    // Drawer balances: USD reduced to 50; THB drawer balance untouched by the 1,600.00 non-cash THB
    let drawer_balances = postings
        .expected_in(&mut tx, &CashPlace::Drawer(shift_id.clone()))
        .await
        .expect("drawer expected");
    assert_eq!(
        drawer_balances.get(usd()),
        money("50.00"),
        "drawer has 50 USD remaining after sell"
    );
    assert_eq!(
        drawer_balances.get(thb()),
        money("0.00"),
        "non-cash THB does not enter drawer"
    );

    // F4 & F3: Assert account entries on Bank and Drawer
    assert_posting_entries(
        &mut tx,
        res.posting_id,
        &[
            ("CUSTOMER", usd(), d("50"), d("1600.00")),
            ("DRAWER", usd(), d("-50"), d("-1600.00")),
            ("BANK", thb(), d("1600.00"), d("1600.00")),
            ("CUSTOMER", thb(), d("-1600.00"), d("-1600.00")),
        ],
    )
    .await;

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// Row 5: A move inside a branch changes no position (§4.4, §9).
/// Asserts inventory position and LG-1 entries (F4).
#[tokio::test]
async fn move_inside_branch_changes_no_position() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();
    let inventory = PgInventoryRepository::new(pool.clone());

    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "3200.00").await;

    // Seed 100 USD position
    let buy_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("seed-pos"),
        line: Some(Uuid::new_v4()),
        reverses: None,
    };
    let buy_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3200.00"),
        source: source("seed-pos"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[buy_usd, buy_thb], None)
        .await
        .expect("seed");

    let pos_before = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("pos before");

    // Move 50 USD from Drawer to Safe inside the same branch
    let move_req = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Safe(branch.clone()),
        currency: usd(),
        quantity: fa("50"),
        source: source("move-internal"),
        line: None,
        reverses: None,
    };
    let res = postings
        .post_in(&mut tx, &[move_req], None)
        .await
        .expect("move within branch");

    let pos_after = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("pos after");

    assert_eq!(
        (pos_after.quantity, pos_after.cost_thb),
        (pos_before.quantity, pos_before.cost_thb),
        "move inside branch leaves inventory position untouched"
    );

    // LG-1 account entries (F4)
    assert_posting_entries(
        &mut tx,
        res.posting_id,
        &[
            ("DRAWER", usd(), d("-50"), d("0.00")),
            ("SAFE", usd(), d("50"), d("0.00")),
        ],
    )
    .await;

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// Row 6: §4.6 after the average has moved: buy 100 USD at 32.00, buy 100 at 34.00,
/// then a reverses request on the first buy disposes 3,200.00 (not the average 3,300.00)
/// and leaves 100 at 3,400.00.
#[tokio::test]
async fn reversal_after_average_moved_disposes_original_cost() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();
    let inventory = PgInventoryRepository::new(pool.clone());

    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "6600.00").await;

    // Buy 1: 100 USD at 32.00 (cost 3,200.00)
    let line_1_id = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "BUY",
        usd(),
        d("100"),
        d("3200.00"),
        Some(d("3200.00")),
        None,
    )
    .await;
    let req_buy1_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("buy-1"),
        line: Some(line_1_id),
        reverses: None,
    };
    let req_buy1_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3200.00"),
        source: source("buy-1"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_buy1_usd, req_buy1_thb], None)
        .await
        .expect("post buy 1");

    // Buy 2: 100 USD at 34.00 (cost 3,400.00)
    let line_2_id = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "BUY",
        usd(),
        d("100"),
        d("3400.00"),
        Some(d("3400.00")),
        None,
    )
    .await;
    let req_buy2_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("buy-2"),
        line: Some(line_2_id),
        reverses: None,
    };
    let req_buy2_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3400.00"),
        source: source("buy-2"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[req_buy2_usd, req_buy2_thb], None)
        .await
        .expect("post buy 2");

    // Position is now 200 USD at 6,600.00 (average 33.00)
    // Reverse the first buy: disposes 3,200.00, not the average 3,300.00, leaves 100 at 3,400.00
    let rev_usd = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: usd(),
        quantity: fa("100"),
        source: source_reversal("rev-buy1"),
        line: None,
        reverses: Some(line_1_id),
    };
    let rev_thb = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: thb(),
        quantity: fa("3200.00"),
        source: source_reversal("rev-buy1"),
        line: None,
        reverses: None,
    };

    let res: PostingResult = postings
        .post_in(&mut tx, &[rev_usd, rev_thb], None)
        .await
        .expect("post reversal of buy 1");

    assert_eq!(res.reversed.len(), 1);
    let rev: &ReversedCost = &res.reversed[0];
    assert_eq!(rev.reverses, line_1_id);
    assert_eq!(
        rev.cost_thb,
        money("3200.00"),
        "disposes 3,200.00 not the average 3,300.00"
    );
    assert_eq!(
        res.reversed[0].difference_thb,
        delta("0.00"),
        "no difference when partial position remains"
    );

    let pos = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("pos after reversal");
    assert_eq!(pos.quantity, d("100"), "leaves 100 in stock");
    assert_eq!(pos.cost_thb, d("3400.00"), "leaves 100 at 3,400.00");

    // R3: §4.6 "the foreign entries carry ±C": ±3,200.00, not the average ±3,300.00.
    assert_posting_entries(
        &mut tx,
        res.posting_id,
        &[
            ("CUSTOMER", usd(), d("100"), d("3200.00")),
            ("DRAWER", usd(), d("-100"), d("-3200.00")),
            ("CUSTOMER", thb(), d("-3200.00"), d("-3200.00")),
            ("DRAWER", thb(), d("3200.00"), d("3200.00")),
        ],
    )
    .await;

    tx.commit().await.expect("commit");
    pool.close().await;
}

/// Row 7: A reversed SELL acquires a lot at the original `cost_basis_thb`, whatever the average is (§4.6, §9).
/// Asserts return values, inventory position, and LG-1 entries (F4).
#[tokio::test]
async fn reversed_sell_acquires_lot_at_original_cost_basis() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();
    let inventory = PgInventoryRepository::new(pool.clone());

    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "5200.00").await;

    // Buy 100 USD at 32.00 (cost 3,200.00)
    let init_buy_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("init-buy"),
        line: Some(Uuid::new_v4()),
        reverses: None,
    };
    let init_buy_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3200.00"),
        source: source("init-buy"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[init_buy_usd, init_buy_thb], None)
        .await
        .expect("init buy");

    // Sell 50 USD with cost basis 1,600.00 (50 @ 32.00)
    let sell_line_id = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("50"),
        d("1700.00"),
        Some(d("1600.00")),
        Some(d("100.00")),
    )
    .await;
    let sell_usd = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: usd(),
        quantity: fa("50"),
        source: source("sell-1"),
        line: Some(sell_line_id),
        reverses: None,
    };
    let sell_thb = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: thb(),
        quantity: fa("1700.00"),
        source: source("sell-1"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[sell_usd, sell_thb], None)
        .await
        .expect("post sell");

    // Buy 50 USD at 40.00 (cost 2,000.00) -> moves the average to (1600 + 2000) / 100 = 36.00
    let buy2_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("50"),
        source: source("buy-expensive"),
        line: Some(Uuid::new_v4()),
        reverses: None,
    };
    let buy2_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("2000.00"),
        source: source("buy-expensive"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[buy2_usd, buy2_thb], None)
        .await
        .expect("post buy 2");

    // Reverse the SELL: acquires 50 USD at original cost basis 1,600.00, whatever the average is
    let rev_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("50"),
        source: source_reversal("rev-sell"),
        line: None,
        reverses: Some(sell_line_id),
    };
    let rev_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("1700.00"),
        source: source_reversal("rev-sell"),
        line: None,
        reverses: None,
    };

    let res = postings
        .post_in(&mut tx, &[rev_usd, rev_thb], None)
        .await
        .expect("reverse sell");

    assert_eq!(res.reversed.len(), 1);
    assert_eq!(res.reversed[0].reverses, sell_line_id);
    assert_eq!(
        res.reversed[0].cost_thb,
        money("1600.00"),
        "acquires at original cost_basis_thb whatever the average is"
    );

    let pos = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("pos");
    assert_eq!(pos.quantity, d("150"));
    assert_eq!(
        pos.cost_thb,
        d("5200.00"),
        "position cost incorporates original cost basis"
    );

    // LG-1 account entries (F4)
    assert_posting_entries(
        &mut tx,
        res.posting_id,
        &[
            ("CUSTOMER", usd(), d("-50"), d("-1600.00")),
            ("DRAWER", usd(), d("50"), d("1600.00")),
            ("CUSTOMER", thb(), d("1700.00"), d("1700.00")),
            ("DRAWER", thb(), d("-1700.00"), d("-1700.00")),
        ],
    )
    .await;

    tx.commit().await.expect("commit");
    pool.close().await;
}

/// Row 8: Buy 100 at 32.00, sell 50, buy 50 at 34.00, reverse the first buy ⇒
/// the position ends at 0 / 0.00 and `difference_thb = 100.00` (§4.6, §9).
/// Asserts return values, inventory position, and LG-1 entries (F4).
#[tokio::test]
async fn reversed_buy_last_unit_rule_and_difference() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();
    let inventory = PgInventoryRepository::new(pool.clone());

    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "4900.00").await;

    // 1. Buy 100 at 32.00 (cost 3,200.00)
    let line_1_id = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "BUY",
        usd(),
        d("100"),
        d("3200.00"),
        Some(d("3200.00")),
        None,
    )
    .await;
    let buy1_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("step-buy-1"),
        line: Some(line_1_id),
        reverses: None,
    };
    let buy1_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3200.00"),
        source: source("step-buy-1"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[buy1_usd, buy1_thb], None)
        .await
        .expect("buy 1");

    // 2. Sell 50 -> remaining 50 at 1,600.00
    let sell_line_id = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("50"),
        d("1650.00"),
        Some(d("1600.00")),
        Some(d("50.00")),
    )
    .await;
    let sell_usd = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: usd(),
        quantity: fa("50"),
        source: source("step-sell-50"),
        line: Some(sell_line_id),
        reverses: None,
    };
    let sell_thb = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: thb(),
        quantity: fa("1650.00"),
        source: source("step-sell-50"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[sell_usd, sell_thb], None)
        .await
        .expect("sell 50");

    // 3. Buy 50 at 34.00 (cost 1,700.00) -> position has 100 USD, cost 1,600 + 1,700 = 3,300.00
    let buy2_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("50"),
        source: source("step-buy-2"),
        line: Some(Uuid::new_v4()),
        reverses: None,
    };
    let buy2_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("1700.00"),
        source: source("step-buy-2"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[buy2_usd, buy2_thb], None)
        .await
        .expect("buy 2");

    // 4. Reverse the first buy (100 USD)
    // The position ends at 0 / 0.00 and difference_thb = 100.00 (3,300 remaining - 3,200 original)
    let rev_usd = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: usd(),
        quantity: fa("100"),
        source: source_reversal("rev-step-buy1"),
        line: None,
        reverses: Some(line_1_id),
    };
    let rev_thb = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: thb(),
        quantity: fa("3200.00"),
        source: source_reversal("rev-step-buy1"),
        line: None,
        reverses: None,
    };

    let res = postings
        .post_in(&mut tx, &[rev_usd, rev_thb], None)
        .await
        .expect("reverse first buy");

    assert_eq!(res.reversed.len(), 1);
    assert_eq!(res.reversed[0].reverses, line_1_id);
    assert_eq!(res.reversed[0].cost_thb, money("3200.00"));
    assert_eq!(
        res.reversed[0].difference_thb,
        delta("100.00"),
        "last unit rule leaves no crumb: difference_thb = 100.00"
    );

    let pos = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("final pos");
    assert_eq!(
        (pos.quantity, pos.cost_thb),
        (d("0"), d("0.00")),
        "position ends at 0 / 0.00"
    );

    // LG-1 account entries (F4)
    assert_posting_entries(
        &mut tx,
        res.posting_id,
        &[
            ("CUSTOMER", usd(), d("100"), d("3200.00")),
            ("DRAWER", usd(), d("-100"), d("-3200.00")),
            ("CUSTOMER", thb(), d("-3200.00"), d("-3200.00")),
            ("DRAWER", thb(), d("3200.00"), d("3200.00")),
        ],
    )
    .await;

    tx.commit().await.expect("commit");
    pool.close().await;
}

/// Row 9: `line` and `reverses` together give `validationFailed` (§4.1, §4.6, §9).
#[tokio::test]
async fn line_and_reverses_together_refused() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");

    let conflicting_req = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: usd(),
        quantity: fa("10"),
        source: source("bad-req"),
        line: Some(Uuid::new_v4()),
        reverses: Some(Uuid::new_v4()),
    };

    let result = postings.post_in(&mut tx, &[conflicting_req], None).await;

    match result {
        Err(AppError::Domain(msg)) => {
            assert!(
                msg.contains("validationFailed"),
                "line and reverses together must refuse with validationFailed"
            );
        }
        _other => panic!("expected AppError::Domain(validationFailed), got unexpected outcome"),
    }

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// Row 10: The original line is unchanged (§4.6, §9).
#[tokio::test]
async fn reversal_leaves_original_line_unchanged() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "3200.00").await;

    let line_id = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "BUY",
        usd(),
        d("100"),
        d("3200.00"),
        Some(d("3200.00")),
        None,
    )
    .await;

    // Perform buy posting
    let buy_usd = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("100"),
        source: source("orig-buy"),
        line: Some(line_id),
        reverses: None,
    };
    let buy_thb = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: thb(),
        quantity: fa("3200.00"),
        source: source("orig-buy"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[buy_usd, buy_thb], None)
        .await
        .expect("buy");

    // Perform reversal
    let rev_usd = PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Customer,
        currency: usd(),
        quantity: fa("100"),
        source: source_reversal("rev-orig"),
        line: None,
        reverses: Some(line_id),
    };
    let rev_thb = PostingRequest {
        from: CashPlace::Customer,
        to: CashPlace::Drawer(shift_id.clone()),
        currency: thb(),
        quantity: fa("3200.00"),
        source: source_reversal("rev-orig"),
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[rev_usd, rev_thb], None)
        .await
        .expect("reverse");

    // Query line from database to verify it was NOT rewritten (§4.6: writes nothing on transaction_lines)
    let row = sqlx::query(
        "SELECT cost_basis_thb, realized_margin_thb, quantity, thb_amount FROM transaction_lines WHERE id = $1",
    )
    .bind(line_id)
    .fetch_one(&mut *tx)
    .await
    .expect("fetch transaction line");

    let cost_after: Decimal = row.get("cost_basis_thb");
    let margin_after: Option<Decimal> = row.get("realized_margin_thb");
    let qty_after: Decimal = row.get("quantity");
    let thb_after: Decimal = row.get("thb_amount");

    assert_eq!(
        cost_after,
        d("3200.00"),
        "original line's cost_basis_thb is never rewritten"
    );
    assert_eq!(
        margin_after, None,
        "original line's realized_margin_thb is untouched"
    );
    assert_eq!(qty_after, d("100"), "original line's quantity is untouched");
    assert_eq!(
        thb_after,
        d("3200.00"),
        "original line's thb_amount is untouched"
    );

    tx.commit().await.expect("commit");
    pool.close().await;
}

/// The transaction a seeded line belongs to: the exchange's `source_id` and the object an approval names.
async fn transaction_of(pool: &PgPool, line: TransactionLineId) -> String {
    sqlx::query_scalar("SELECT transaction_id::text FROM transaction_lines WHERE id = $1")
        .bind(line)
        .fetch_one(pool)
        .await
        .expect("the line's transaction")
}

/// An approval request through `PgApprovalRepository` (F6), naming `object_id` at the hash the
/// implementation computes now over that source's lines (lane controller's ruling, issuecomment-6097817311:
/// #924 implements `NegativeStockSource` over the source lines; here `LedgerCashPostings` is that source).
/// `status` is PENDING (as inserted), APPROVED or REJECTED.
async fn approval(
    pool: &PgPool,
    postings: &LedgerCashPostings,
    branch: &BranchId,
    object_id: &str,
    type_name: &str,
    status: &'static str,
) -> ApprovalRequestId {
    let repo = PgApprovalRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    let hash = NegativeStockSource::version_hash(postings, &mut tx, object_id)
        .await
        .expect("the source's version hash");
    let req = repo
        .insert_request(
            &mut tx,
            type_name,
            Some(branch.as_str().to_string()),
            object_id,
            &hash,
            "v1",
            serde_json::json!({}),
            "teller",
            &unique("key"),
        )
        .await
        .expect("insert approval request");
    if status != "PENDING" {
        repo.update_status(&mut tx, &req.id, status)
            .await
            .expect("decide the approval request");
    }
    tx.commit().await.expect("commit");
    ApprovalRequestId::new(req.id.as_str())
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

/// The posting is refused `stockInsufficient` and writes nothing (§4.5). Runs in a savepoint so the
/// test can go on after it.
async fn assert_refused_writing_nothing(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    postings: &LedgerCashPostings,
    reqs: &[PostingRequest],
    allow_negative: Option<ApprovalRequestId>,
    what: &str,
) {
    let mut sp = Connection::begin(&mut **tx).await.expect("savepoint");
    let before = written(&mut sp).await;
    match postings.post_in(&mut sp, reqs, allow_negative).await {
        Err(AppError::Domain(msg)) if msg.contains("stockInsufficient") => {}
        other => panic!("{what}: expected stockInsufficient, got {other:?}"),
    }
    assert_eq!(written(&mut sp).await, before, "{what}: nothing is written");
    sp.rollback().await.expect("rollback savepoint");
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

fn buy_legs(
    shift_id: &str,
    line: TransactionLineId,
    qty: &str,
    thb_out: &str,
    src: SourceRef,
) -> [PostingRequest; 2] {
    [
        PostingRequest {
            from: CashPlace::Customer,
            to: CashPlace::Drawer(shift_id.to_string()),
            currency: usd(),
            quantity: fa(qty),
            source: src.clone(),
            line: Some(line),
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

async fn line_figures(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    line: TransactionLineId,
) -> (Option<Decimal>, Option<Decimal>, Decimal, Decimal, String) {
    let row = sqlx::query(
        "SELECT cost_basis_thb, realized_margin_thb, quantity, thb_amount, updated_at::text AS updated \
         FROM transaction_lines WHERE id = $1",
    )
    .bind(line)
    .fetch_one(&mut **tx)
    .await
    .expect("fetch transaction line");
    (
        row.get("cost_basis_thb"),
        row.get("realized_margin_thb"),
        row.get("quantity"),
        row.get("thb_amount"),
        row.get("updated"),
    )
}

/// Row 11: Q-4 (§4.5; lane controller's ruling A, issuecomment-6097795288 [required]; scenario of
/// issuecomment-6097791219). Buy 100 at 32.00 and sell 100 (position 0, last `average_after` 32); sell 10
/// more with an APPROVED `NEGATIVE_STOCK` (provisional cost 320.00, position −10 / −320.00); buy 100 at
/// 34.00. Exactly one cost-only `ADJUST`: `quantity_signed = 0`, `cost_thb_signed = −20.00`,
/// `transaction_line_id` = the short sale's line, `average_after = 34.00000000`. The position is
/// 90 / 3,060.00, and the short sale's `transaction_lines` row is unchanged by the buy.
#[tokio::test]
async fn negative_stock_provisional_and_q4_adjust() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;

    let postings = LedgerCashPostings::new();
    let inventory = PgInventoryRepository::new(pool.clone());

    let mut tx = pool.begin().await.expect("begin");
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "10000.00").await;

    // 1. Buy 100 at 32.00.
    postings
        .post_in(
            &mut tx,
            &buy_legs(
                &shift_id,
                Uuid::new_v4(),
                "100",
                "3200.00",
                source("q4-buy-32"),
            ),
            None,
        )
        .await
        .expect("buy 100 at 32.00");

    // 2. Sell 100: the position is 0 and the last average_after is 32.
    let sell_all = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("100"),
        d("3300.00"),
        None,
        None,
    )
    .await;
    postings
        .post_in(
            &mut tx,
            &sell_legs(&shift_id, sell_all, "100", "3300.00", source("q4-sell-100")),
            None,
        )
        .await
        .expect("sell 100");
    let pos_zero = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("position after selling all");
    assert_eq!((pos_zero.quantity, pos_zero.cost_thb), (d("0"), d("0.00")));

    // 3. Sell 10 more with an APPROVED NEGATIVE_STOCK for this source.
    let short_line = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("10"),
        d("330.00"),
        None,
        None,
    )
    .await;
    let short_tx = transaction_of(&pool, short_line).await;
    let approved = approval(
        &pool,
        &postings,
        &branch,
        &short_tx,
        "NEGATIVE_STOCK",
        "APPROVED",
    )
    .await;
    let res_short = postings
        .post_in(
            &mut tx,
            &sell_legs(&shift_id, short_line, "10", "330.00", source(&short_tx)),
            Some(approved),
        )
        .await
        .expect("approved short sale");
    assert_eq!(
        res_short.costs,
        vec![LineCost {
            line: short_line,
            cost_basis_thb: money("320.00"),
            realized_margin_thb: Some(delta("10.00")),
        }],
        "the 10 beyond zero are costed at the last average_after 32: provisional cost 320.00"
    );
    let pos_neg = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("position after the short sale");
    assert_eq!(
        (pos_neg.quantity, pos_neg.cost_thb),
        (d("-10"), d("-320.00")),
        "position −10 / −320.00"
    );
    assert_posting_entries(
        &mut tx,
        res_short.posting_id,
        &[
            ("CUSTOMER", usd(), d("10"), d("320.00")),
            ("DRAWER", usd(), d("-10"), d("-320.00")),
            ("CUSTOMER", thb(), d("-330.00"), d("-330.00")),
            ("DRAWER", thb(), d("330.00"), d("330.00")),
        ],
    )
    .await;
    let short_line_before = line_figures(&mut tx, short_line).await;

    // 4. Buy 100 at 34.00.
    let res_buy = postings
        .post_in(
            &mut tx,
            &buy_legs(
                &shift_id,
                Uuid::new_v4(),
                "100",
                "3400.00",
                source("q4-buy-34"),
            ),
            None,
        )
        .await
        .expect("buy 100 at 34.00");
    // The ADJUST is a cost-only movement: it adds no ledger entry to the buy's posting.
    assert_posting_entries(
        &mut tx,
        res_buy.posting_id,
        &[
            ("CUSTOMER", usd(), d("-100"), d("-3400.00")),
            ("DRAWER", usd(), d("100"), d("3400.00")),
            ("CUSTOMER", thb(), d("3400.00"), d("3400.00")),
            ("DRAWER", thb(), d("-3400.00"), d("-3400.00")),
        ],
    )
    .await;

    let adjusts: Vec<(Decimal, Decimal, Option<Uuid>, Option<Decimal>)> = sqlx::query_as(
        "SELECT quantity_signed, cost_thb_signed, transaction_line_id, average_after \
         FROM inventory_movements \
         WHERE organization_id = $1::uuid AND branch_id = $2 AND currency = 'USD' AND movement_type = 'ADJUST'",
    )
    .bind(ORG)
    .bind(branch.as_str())
    .fetch_all(&mut *tx)
    .await
    .expect("fetch ADJUST movements");
    assert_eq!(
        adjusts,
        vec![(
            d("0"),
            d("-20.00"),
            Some(short_line),
            Some(d("34.00000000"))
        )],
        "ruling A: exactly one cost-only ADJUST (quantity 0, −20.00, the short sale's line, average 34)"
    );

    let pos_after = inventory
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("position after the buy");
    assert_eq!(
        (pos_after.quantity, pos_after.cost_thb),
        (d("90"), d("3060.00")),
        "position 90 / 3,060.00: the average restarts from the real cost 34.00"
    );
    assert_eq!(
        line_figures(&mut tx, short_line).await,
        short_line_before,
        "§4.3: the short sale's transaction_lines row is not rewritten by the buy"
    );

    tx.commit().await.expect("commit");
    pool.close().await;
}

/// Row 12 (R4a; §4.5, the ticket's negative-stock behaviour): a posting that takes a place or the
/// position below zero, without `allow_negative`, is refused `stockInsufficient` and writes nothing.
#[tokio::test]
async fn below_zero_without_allow_negative_refused_writing_nothing() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");

    // The position and the drawer are both 0: a SELL of 10.
    let line = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("10"),
        d("330.00"),
        None,
        None,
    )
    .await;
    let sell_tx = transaction_of(&pool, line).await;
    assert_refused_writing_nothing(
        &mut tx,
        &postings,
        &sell_legs(&shift_id, line, "10", "330.00", source(&sell_tx)),
        None,
        "a SELL beyond a zero position and an empty drawer",
    )
    .await;

    // The branch holds 100 USD, all in the safe: the drawer still may not go below zero.
    fund_drawer_thb(&mut tx, &postings, &branch, &shift_id, "3200.00").await;
    postings
        .post_in(
            &mut tx,
            &buy_legs(
                &shift_id,
                Uuid::new_v4(),
                "100",
                "3200.00",
                source("r4a-buy"),
            ),
            None,
        )
        .await
        .expect("buy 100");
    let to_safe = |qty: &str, id: &str| PostingRequest {
        from: CashPlace::Drawer(shift_id.clone()),
        to: CashPlace::Safe(branch.clone()),
        currency: usd(),
        quantity: fa(qty),
        source: SourceRef {
            source_type: SourceType::Transfer,
            source_id: id.to_string(),
            source_key: "move".to_string(),
        },
        line: None,
        reverses: None,
    };
    postings
        .post_in(&mut tx, &[to_safe("100", "r4a-move")], None)
        .await
        .expect("move the 100 to the safe");
    assert_refused_writing_nothing(
        &mut tx,
        &postings,
        &[to_safe("10", "r4a-drawer-short")],
        None,
        "DRAWER below zero while the position is 100",
    )
    .await;
    let from_safe = PostingRequest {
        from: CashPlace::Safe(branch.clone()),
        to: CashPlace::Drawer(shift_id.clone()),
        currency: usd(),
        quantity: fa("150"),
        source: SourceRef {
            source_type: SourceType::Transfer,
            source_id: "r4a-safe-short".to_string(),
            source_key: "move".to_string(),
        },
        line: None,
        reverses: None,
    };
    assert_refused_writing_nothing(
        &mut tx,
        &postings,
        &[from_safe],
        None,
        "SAFE below zero (150 out of 100)",
    )
    .await;

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// Row 13 (R4b; test-author choice 4, accepted in issuecomment-6097791219): `allow_negative` must name an
/// APPROVED request of type `NEGATIVE_STOCK` whose object is this posting's source. PENDING, REJECTED,
/// another type and another source are each refused `stockInsufficient` and write nothing; the
/// APPROVED one for this source then posts.
#[tokio::test]
async fn allow_negative_needs_an_approved_negative_stock_for_this_source() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();

    let line = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("10"),
        d("330.00"),
        None,
        None,
    )
    .await;
    let sell_tx = transaction_of(&pool, line).await;
    let other_line = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("10"),
        d("330.00"),
        None,
        None,
    )
    .await;
    let other_tx = transaction_of(&pool, other_line).await;

    // One PENDING request per object (`one_pending_per_object`), so the PENDING one is made last.
    let rejected = approval(
        &pool,
        &postings,
        &branch,
        &sell_tx,
        "NEGATIVE_STOCK",
        "REJECTED",
    )
    .await;
    let other_type = approval(
        &pool,
        &postings,
        &branch,
        &sell_tx,
        "COUNT_DIFFERENCE",
        "APPROVED",
    )
    .await;
    let other_source = approval(
        &pool,
        &postings,
        &branch,
        &other_tx,
        "NEGATIVE_STOCK",
        "APPROVED",
    )
    .await;
    let approved = approval(
        &pool,
        &postings,
        &branch,
        &sell_tx,
        "NEGATIVE_STOCK",
        "APPROVED",
    )
    .await;
    let pending = approval(
        &pool,
        &postings,
        &branch,
        &sell_tx,
        "NEGATIVE_STOCK",
        "PENDING",
    )
    .await;

    let mut tx = pool.begin().await.expect("begin");
    let legs = sell_legs(&shift_id, line, "10", "330.00", source(&sell_tx));
    for (request, what) in [
        (pending, "a PENDING request"),
        (rejected, "a REJECTED request"),
        (other_type, "an APPROVED request of another type"),
        (
            other_source,
            "an APPROVED NEGATIVE_STOCK for another source",
        ),
    ] {
        assert_refused_writing_nothing(&mut tx, &postings, &legs, Some(request), what).await;
    }

    let pub_req = Uuid::new_v4();
    let pub_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES ($1, 'RATE_PUBLICATION', $2, $3, 'h', 'p', '{}'::jsonb, 'test', now(), 'APPROVED', $4)",
    )
    .bind(pub_req)
    .bind(branch.as_str())
    .bind(unique("pub-obj"))
    .bind(unique("pub-key"))
    .execute(&pool)
    .await
    .expect("approved rate publication request");
    sqlx::query(
        "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
         VALUES ($1, $2, 1, now(), $3, 'test')",
    )
    .bind(pub_id)
    .bind(branch.as_str())
    .bind(pub_req)
    .execute(&pool)
    .await
    .expect("rate publication");
    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, \
         value_tier, unit_size, buy_rate, sell_rate, source_snapshot) \
         VALUES ($1, $2, 'USD', 'ALL', 'ALL', 1, 32.50, 33.00, '{}'::jsonb)",
    )
    .bind(Uuid::new_v4())
    .bind(pub_id)
    .execute(&pool)
    .await
    .expect("published rate line");
    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(pub_id)
        .bind(branch.as_str())
        .execute(&pool)
        .await
        .expect("current publication");

    postings
        .post_in(&mut tx, &legs, Some(approved))
        .await
        .expect("the APPROVED NEGATIVE_STOCK for this source allows the posting");
    let pos = PgInventoryRepository::new(pool.clone())
        .position_for_update_in(&mut tx, ORG, &branch, usd())
        .await
        .expect("position");
    assert_eq!(
        pos.quantity,
        d("-10"),
        "the approved posting takes the position below zero"
    );

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// Row 14 (R4c; lane controller's ruling, issuecomment-6097817311 [required]): the hash is computed over
/// the posting's source lines. A request approved before a source line changed does not allow the
/// posting (`stockInsufficient`, nothing written); one approved at the changed lines does.
#[tokio::test]
async fn approval_for_changed_source_lines_refused() {
    let (pool, branch) = db().await;
    let shift_id = seed_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();

    let line = seed_transaction_line(
        &pool,
        &branch,
        &shift_id,
        "SELL",
        usd(),
        d("10"),
        d("330.00"),
        None,
        None,
    )
    .await;
    let sell_tx = transaction_of(&pool, line).await;
    let stale = approval(
        &pool,
        &postings,
        &branch,
        &sell_tx,
        "NEGATIVE_STOCK",
        "APPROVED",
    )
    .await;

    sqlx::query(
        "UPDATE transaction_lines SET thb_amount = 340.00, settle_thb = 340.00 WHERE id = $1",
    )
    .bind(line)
    .execute(&pool)
    .await
    .expect("change the source line");

    let mut tx = pool.begin().await.expect("begin");
    let legs = sell_legs(&shift_id, line, "10", "340.00", source(&sell_tx));
    assert_refused_writing_nothing(
        &mut tx,
        &postings,
        &legs,
        Some(stale),
        "an approval given before the source line changed",
    )
    .await;
    tx.rollback().await.expect("rollback");

    let fresh = approval(
        &pool,
        &postings,
        &branch,
        &sell_tx,
        "NEGATIVE_STOCK",
        "APPROVED",
    )
    .await;
    let pub_req = Uuid::new_v4();
    let pub_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES ($1, 'RATE_PUBLICATION', $2, $3, 'h', 'p', '{}'::jsonb, 'test', now(), 'APPROVED', $4)",
    )
    .bind(pub_req)
    .bind(branch.as_str())
    .bind(unique("pub-obj"))
    .bind(unique("pub-key"))
    .execute(&pool)
    .await
    .expect("approved rate publication request");
    sqlx::query(
        "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
         VALUES ($1, $2, 1, now(), $3, 'test')",
    )
    .bind(pub_id)
    .bind(branch.as_str())
    .bind(pub_req)
    .execute(&pool)
    .await
    .expect("rate publication");
    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, \
         value_tier, unit_size, buy_rate, sell_rate, source_snapshot) \
         VALUES ($1, $2, 'USD', 'ALL', 'ALL', 1, 32.50, 33.00, '{}'::jsonb)",
    )
    .bind(Uuid::new_v4())
    .bind(pub_id)
    .execute(&pool)
    .await
    .expect("published rate line");
    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(pub_id)
        .bind(branch.as_str())
        .execute(&pool)
        .await
        .expect("current publication");
    let mut tx = pool.begin().await.expect("begin");
    postings
        .post_in(&mut tx, &legs, Some(fresh))
        .await
        .expect("an approval at the changed lines allows the posting");
    tx.rollback().await.expect("rollback");
    pool.close().await;
}
