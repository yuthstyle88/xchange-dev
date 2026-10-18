//! Up-front tests for LG-6.0 (#1039): the `ShiftBalances` read port (`xc-shift`) and
//! `PgLedgerShiftBalances` (`xc-cash`), design `docs/design/blueprint/ledger-inventory.md` §4.2, §4.2.1
//! (PR 1037; `expected` signed per PR 1458 and the lane controller's Q1/Q2 rulings on #1039).
//! Written by claude/dell-claude-3 as test-writer, ordered by claude/ad05bf8d.
//!
//! Every figure is posted through the real port (`LedgerCashPostings`, and `post_opening_in` /
//! `post_difference_in`, the seams the OPEN count, the hand-over and the CLOSE count use). Counts are
//! seeded as rows, because `ShiftBalances` only reads `cash_counts` / `cash_count_totals`.
//! `PgLedgerShiftBalances` reads on its own pool, so each posting is committed before it is read.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
//! Each test gets a fresh schema with every migration of `api/migrations` applied.

use std::collections::{BTreeMap, HashSet};
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
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_cash::application::count::count_service::{post_difference_in, post_opening_in};
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, PostingRequest, TransactionLineId,
};
use xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, MoneyDelta};
use xc_shift::domain::shift::repositories::shift_balances::{ShiftBalance, ShiftBalances};
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

fn delta(s: &str) -> MoneyDelta {
    let minor = (d(s) * Decimal::from(100)).round().to_i128().expect("fits");
    MoneyDelta::from_minor(minor)
}

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).unwrap_or_else(|_| panic!("a currency code: {s}"))
}

fn thb() -> CurrencyCode {
    code("THB")
}

fn usd() -> CurrencyCode {
    code("USD")
}

fn eur() -> CurrencyCode {
    code("EUR")
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
    let schema = unique("lg6sb");
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

/// A new OPEN shift on `branch`. A branch has one OPEN shift at a time (`shifts_one_open_per_branch_v2`),
/// so the branch's open shift is closed first, as v2 does. The port does not look at a shift's status.
async fn next_shift(pool: &PgPool, branch: &BranchId) -> String {
    sqlx::query(
        "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'",
    )
    .bind(branch.as_str())
    .execute(pool)
    .await
    .expect("close the open shift");
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

fn src(source_type: SourceType, id: &str, key: &str) -> SourceRef {
    SourceRef {
        source_type,
        source_id: id.to_string(),
        source_key: key.to_string(),
    }
}

fn req(
    from: CashPlace,
    to: CashPlace,
    currency: CurrencyCode,
    quantity: &str,
    source: &SourceRef,
    line: Option<TransactionLineId>,
) -> PostingRequest {
    PostingRequest {
        from,
        to,
        currency,
        quantity: fa(quantity),
        source: source.clone(),
        line,
        reverses: None,
    }
}

fn drawer(shift: &str) -> CashPlace {
    CashPlace::Drawer(shift.to_string())
}

/// One committed posting through the port.
async fn post(
    pool: &PgPool,
    ledger: &LedgerCashPostings,
    reqs: &[PostingRequest],
    allow_negative: Option<ApprovalRequestId>,
    what: &str,
) {
    let mut tx = pool.begin().await.expect("begin");
    ledger
        .post_in(&mut tx, reqs, allow_negative)
        .await
        .unwrap_or_else(|e| panic!("{what}: the port refused: {e:?}"));
    tx.commit().await.expect("commit");
}

/// The branch's safe holds `amounts` (`BANK → SAFE`, as the LG-4 harness funds it), so a drawer can
/// open from it (§4.5 refuses a safe below zero).
async fn fund_safe(
    pool: &PgPool,
    ledger: &LedgerCashPostings,
    branch: &BranchId,
    amounts: &[(CurrencyCode, &str)],
) {
    for (c, a) in amounts {
        let s = src(SourceType::Opening, &unique("fund"), "fund");
        post(
            pool,
            ledger,
            &[req(
                CashPlace::Bank,
                CashPlace::Safe(branch.clone()),
                *c,
                a,
                &s,
                None,
            )],
            None,
            "fund the safe",
        )
        .await;
    }
}

/// Opens `DRAWER(shift)` from `from` through `post_opening_in` (the OPEN count's and the hand-over's seam):
/// one `OPENING` posting with an entry pair per currency (§4.4 rows 1 and 2).
async fn open_from(
    pool: &PgPool,
    ledger: &LedgerCashPostings,
    from: CashPlace,
    shift: &str,
    amounts: &[(CurrencyCode, &str)],
) {
    let amounts: Vec<(CurrencyCode, Decimal)> = amounts.iter().map(|(c, a)| (*c, d(a))).collect();
    let mut tx = pool.begin().await.expect("begin");
    post_opening_in(ledger, &mut tx, from, shift, &amounts, &unique("open"))
        .await
        .expect("open the drawer through post_opening_in");
    tx.commit().await.expect("commit");
}

/// A cash exchange on `shift`: the foreign leg with a line, the THB leg in the drawer (§4.3).
fn exchange(
    shift: &str,
    side: &str,
    currency: CurrencyCode,
    qty: &str,
    thb_amount: &str,
    id: &str,
) -> Vec<PostingRequest> {
    let s = src(SourceType::Exchange, id, "complete");
    let line = Some(Uuid::new_v4());
    if side == "BUY" {
        vec![
            req(CashPlace::Customer, drawer(shift), currency, qty, &s, line),
            req(
                drawer(shift),
                CashPlace::Customer,
                thb(),
                thb_amount,
                &s,
                None,
            ),
        ]
    } else {
        vec![
            req(drawer(shift), CashPlace::Customer, currency, qty, &s, line),
            req(
                CashPlace::Customer,
                drawer(shift),
                thb(),
                thb_amount,
                &s,
                None,
            ),
        ]
    }
}

/// A drawer ↔ safe move (`TRANSFER`, §4.4 row 3).
fn transfer(
    branch: &BranchId,
    shift: &str,
    currency: CurrencyCode,
    qty: &str,
    into_drawer: bool,
) -> PostingRequest {
    let s = src(SourceType::Transfer, &unique("move"), "move");
    let safe = CashPlace::Safe(branch.clone());
    if into_drawer {
        req(safe, drawer(shift), currency, qty, &s, None)
    } else {
        req(drawer(shift), safe, currency, qty, &s, None)
    }
}

/// A count row on `shift`, with its totals `(currency, counted, expected, difference)`. `verified` sets
/// `verified_by` (another user than the counter) and `verified_at`.
async fn seed_count(
    pool: &PgPool,
    branch: &BranchId,
    shift: &str,
    kind: &str,
    verified: bool,
    totals: &[(CurrencyCode, &str, &str, &str)],
) {
    let counter = seed_user(pool).await;
    let verifier = if verified {
        Some(seed_user(pool).await)
    } else {
        None
    };
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, \
         verified_by, verified_at, idempotency_key) \
         VALUES ($1::uuid, $2, $3, $4, $5, now(), $6, CASE WHEN $6::uuid IS NULL THEN NULL ELSE now() END, $7) \
         RETURNING id",
    )
    .bind(ORG)
    .bind(branch.as_str())
    .bind(shift)
    .bind(kind)
    .bind(counter)
    .bind(verifier)
    .bind(unique("count"))
    .fetch_one(pool)
    .await
    .expect("insert cash_counts");
    for (c, counted, expected, difference) in totals {
        sqlx::query(
            "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(ORG)
        .bind(c.as_str())
        .bind(d(counted))
        .bind(d(expected))
        .bind(d(difference))
        .execute(pool)
        .await
        .expect("insert cash_count_totals");
    }
}

async fn balances_of(sb: &PgLedgerShiftBalances, shift: &str) -> Vec<ShiftBalance> {
    sb.shift_balances(&shift.to_string())
        .await
        .unwrap_or_else(|e| panic!("shift_balances({shift}): {e:?}"))
}

fn row(rows: &[ShiftBalance], c: CurrencyCode) -> &ShiftBalance {
    rows.iter()
        .find(|r| r.currency == c)
        .unwrap_or_else(|| panic!("no {c} row in {rows:?}"))
}

fn currencies(rows: &[ShiftBalance]) -> Vec<String> {
    rows.iter()
        .map(|r| r.currency.as_str().to_string())
        .collect()
}

async fn expected_in(pool: &PgPool, ledger: &LedgerCashPostings, shift: &str) -> Balances {
    let mut tx = pool.begin().await.expect("begin");
    let b = ledger
        .expected_in(&mut tx, &drawer(shift))
        .await
        .expect("expected_in(Drawer(shift))");
    tx.rollback().await.expect("rollback");
    b
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 row 1: a shift with no posting and no count
// ─────────────────────────────────────────────────────────────────────────────

/// §9 row 1: a shift with no posting and no count answers `[]`. Another shift of the branch that has a
/// posting and a verified count is there, so the rows must be the asked shift's only.
#[tokio::test]
async fn a_shift_with_no_posting_and_no_count_answers_nothing() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(&pool, &ledger, &branch, &[(thb(), "100000")]).await;

    let busy = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &busy,
        &[(thb(), "5000")],
    )
    .await;
    seed_count(
        &pool,
        &branch,
        &busy,
        "CLOSE",
        true,
        &[(thb(), "5000", "5000", "0")],
    )
    .await;
    let empty = next_shift(&pool, &branch).await;

    assert_eq!(
        balances_of(&sb, &empty).await,
        Vec::<ShiftBalance>::new(),
        "a shift with no posting and no count has no balances"
    );
    assert_eq!(
        currencies(&balances_of(&sb, &busy).await),
        ["THB"],
        "the other shift still answers its own row"
    );
    pool.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 row 2 and §8 counted/difference
// ─────────────────────────────────────────────────────────────────────────────

/// §9 row 2: a CLOSE count submitted but not verified gives `counted: None`, `difference: None`; the
/// drawer's other figures are still there.
#[tokio::test]
async fn an_unverified_close_count_gives_no_counted_and_no_difference() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(
        &pool,
        &ledger,
        &branch,
        &[(thb(), "100000"), (usd(), "1000")],
    )
    .await;
    let shift = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &shift,
        &[(thb(), "5000"), (usd(), "100")],
    )
    .await;
    seed_count(
        &pool,
        &branch,
        &shift,
        "CLOSE",
        false,
        &[(thb(), "4990", "5000", "-10"), (usd(), "100", "100", "0")],
    )
    .await;

    let rows = balances_of(&sb, &shift).await;
    assert_eq!(currencies(&rows), ["THB", "USD"]);
    for c in [thb(), usd()] {
        let r = row(&rows, c);
        assert_eq!(r.counted, None, "{c}: an unverified count is not counted");
        assert_eq!(
            r.difference, None,
            "{c}: an unverified count has no difference"
        );
    }
    assert_eq!(row(&rows, thb()).expected, delta("5000"));
    assert_eq!(row(&rows, usd()).expected, delta("100"));
    pool.close().await;
}

/// §8 `counted` / `difference`: from `cash_count_totals` of the shift's **verified** CLOSE count, or its
/// verified HANDOVER_OUT count. A verified OPEN count, and another shift's verified CLOSE count, are not
/// it. A verified count line in a currency the drawer never held still answers a row (§4.2.1 "a shift
/// with no posting yet answers its count rows").
#[tokio::test]
async fn counted_and_difference_come_from_the_shifts_verified_close_or_handover_out_count() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(
        &pool,
        &ledger,
        &branch,
        &[(thb(), "100000"), (usd(), "1000")],
    )
    .await;

    // Shift A: closed with a verified CLOSE count; its verified OPEN count must not be read. The OPEN
    // count also carries a GBP line the drawer never held, so reading it would add a GBP row.
    let a = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &a,
        &[(thb(), "5000"), (usd(), "100")],
    )
    .await;
    seed_count(
        &pool,
        &branch,
        &a,
        "OPEN",
        true,
        &[
            (thb(), "5000", "5000", "0"),
            (usd(), "100", "100", "0"),
            (code("GBP"), "5", "5", "0"),
        ],
    )
    .await;
    seed_count(
        &pool,
        &branch,
        &a,
        "CLOSE",
        true,
        &[
            (thb(), "4990.50", "5000", "-9.50"),
            (usd(), "101", "100", "1"),
            (eur(), "20", "0", "20"),
        ],
    )
    .await;

    // Shift B: handed over, with a verified HANDOVER_OUT count.
    let b = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &b,
        &[(thb(), "3000")],
    )
    .await;
    seed_count(
        &pool,
        &branch,
        &b,
        "HANDOVER_OUT",
        true,
        &[(thb(), "3000", "3000", "0")],
    )
    .await;

    // Shift C: only a verified count, no posting at all.
    let c = next_shift(&pool, &branch).await;
    seed_count(&pool, &branch, &c, "CLOSE", true, &[(usd(), "7", "0", "7")]).await;

    let rows_a = balances_of(&sb, &a).await;
    assert_eq!(currencies(&rows_a), ["THB", "EUR", "USD"]);
    assert_eq!(row(&rows_a, thb()).counted, Some(money("4990.50")));
    assert_eq!(row(&rows_a, thb()).difference, Some(delta("-9.50")));
    assert_eq!(row(&rows_a, usd()).counted, Some(money("101")));
    assert_eq!(row(&rows_a, usd()).difference, Some(delta("1")));
    let eur_a = row(&rows_a, eur());
    assert_eq!(
        (
            eur_a.opening,
            eur_a.expected,
            eur_a.counted,
            eur_a.difference
        ),
        (money("0"), delta("0"), Some(money("20")), Some(delta("20"))),
        "a verified count line in a currency the drawer never held answers a row of its own"
    );

    let rows_b = balances_of(&sb, &b).await;
    assert_eq!(currencies(&rows_b), ["THB"]);
    assert_eq!(row(&rows_b, thb()).counted, Some(money("3000")));
    assert_eq!(row(&rows_b, thb()).difference, Some(delta("0")));

    let rows_c = balances_of(&sb, &c).await;
    assert_eq!(currencies(&rows_c), ["USD"]);
    let usd_c = row(&rows_c, usd());
    assert_eq!(
        (
            usd_c.opening,
            usd_c.expected,
            usd_c.moved_in,
            usd_c.moved_out,
            usd_c.counted,
            usd_c.difference
        ),
        (
            money("0"),
            delta("0"),
            money("0"),
            money("0"),
            Some(money("7")),
            Some(delta("7"))
        ),
        "a shift with no posting answers its count rows only"
    );
    pool.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 row 4 and §8 moved_in / moved_out
// ─────────────────────────────────────────────────────────────────────────────

/// §9 row 4: an exchange (`EXCHANGE`) and a drawer→safe move (`TRANSFER`) on the same shift: the move is in
/// `moved_out`, the exchange is in neither moved figure, and the opening (`OPENING`) is in neither either.
/// A safe→drawer move in THB is in `moved_in` only.
#[tokio::test]
async fn a_transfer_is_a_moved_figure_and_an_exchange_is_not() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(
        &pool,
        &ledger,
        &branch,
        &[(thb(), "100000"), (usd(), "5000")],
    )
    .await;
    let shift = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &shift,
        &[(thb(), "10000"), (usd(), "1000")],
    )
    .await;

    // BUY 100 USD for 3,200.00 THB cash; SELL 40 USD for 1,320.00 THB cash.
    post(
        &pool,
        &ledger,
        &exchange(&shift, "BUY", usd(), "100", "3200", &unique("ex")),
        None,
        "BUY",
    )
    .await;
    post(
        &pool,
        &ledger,
        &exchange(&shift, "SELL", usd(), "40", "1320", &unique("ex")),
        None,
        "SELL",
    )
    .await;
    // Drawer → safe 300 USD; safe → drawer 500 THB.
    post(
        &pool,
        &ledger,
        &[transfer(&branch, &shift, usd(), "300", false)],
        None,
        "USD to the safe",
    )
    .await;
    post(
        &pool,
        &ledger,
        &[transfer(&branch, &shift, thb(), "500", true)],
        None,
        "THB from the safe",
    )
    .await;

    let rows = balances_of(&sb, &shift).await;
    assert_eq!(currencies(&rows), ["THB", "USD"]);
    let u = row(&rows, usd());
    assert_eq!(
        (u.opening, u.expected, u.moved_in, u.moved_out),
        (money("1000"), delta("760"), money("0"), money("300")),
        "USD: 1000 + 100 − 40 − 300; only the TRANSFER is moved (out)"
    );
    let t = row(&rows, thb());
    assert_eq!(
        (t.opening, t.expected, t.moved_in, t.moved_out),
        (money("10000"), delta("8620"), money("500"), money("0")),
        "THB: 10000 − 3200 + 1320 + 500; only the TRANSFER is moved (in)"
    );
    pool.close().await;
}

/// §8 `moved_in` / `moved_out`: the set is every posting whose `source_type` is neither `EXCHANGE` nor
/// `OPENING`, so a verified count's difference (`COUNT`, `DRAWER ↔ VARIANCE`, §4.4 row 7) is a moved
/// figure too: a shortfall out, an overage in.
#[tokio::test]
async fn a_count_difference_is_a_moved_figure() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(
        &pool,
        &ledger,
        &branch,
        &[(thb(), "100000"), (usd(), "5000")],
    )
    .await;
    let shift = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &shift,
        &[(thb(), "10000"), (usd(), "1000")],
    )
    .await;

    let mut tx = pool.begin().await.expect("begin");
    post_difference_in(
        &ledger,
        &mut tx,
        &shift,
        &branch,
        &[(thb(), d("-12.50")), (usd(), d("3"))],
        &unique("diff"),
    )
    .await
    .expect("post the count difference");
    tx.commit().await.expect("commit");

    let rows = balances_of(&sb, &shift).await;
    let t = row(&rows, thb());
    assert_eq!(
        (t.expected, t.moved_in, t.moved_out),
        (delta("9987.50"), money("0"), money("12.50")),
        "THB shortfall: DRAWER → VARIANCE is moved out"
    );
    let u = row(&rows, usd());
    assert_eq!(
        (u.expected, u.moved_in, u.moved_out),
        (delta("1003"), money("3"), money("0")),
        "USD overage: VARIANCE → DRAWER is moved in"
    );
    pool.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 row 5: a hand-over-opened shift
// ─────────────────────────────────────────────────────────────────────────────

/// §9 row 5: a shift opened by a hand-over (`DRAWER(previous) → DRAWER(shift)`, one `OPENING` posting
/// through `post_opening_in`) has `opening` = the hand-over quantity. The giver's opening is unchanged,
/// and the hand-over is in neither shift's moved figures (it is `OPENING`).
#[tokio::test]
async fn a_hand_over_opened_shift_opens_with_the_hand_over_quantity() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(
        &pool,
        &ledger,
        &branch,
        &[(thb(), "100000"), (usd(), "5000")],
    )
    .await;

    let giver = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &giver,
        &[(thb(), "5000"), (usd(), "200")],
    )
    .await;
    post(
        &pool,
        &ledger,
        &exchange(&giver, "BUY", usd(), "50", "1600", &unique("ex")),
        None,
        "the giver's BUY",
    )
    .await;

    let taker = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        drawer(&giver),
        &taker,
        &[(thb(), "3400"), (usd(), "250")],
    )
    .await;

    let rows = balances_of(&sb, &taker).await;
    assert_eq!(currencies(&rows), ["THB", "USD"]);
    for (c, q) in [(thb(), "3400"), (usd(), "250")] {
        let r = row(&rows, c);
        assert_eq!(
            (r.opening, r.expected, r.moved_in, r.moved_out),
            (money(q), delta(q), money("0"), money("0")),
            "{c}: the taker opens with the hand-over quantity"
        );
    }

    let given = balances_of(&sb, &giver).await;
    let t = row(&given, thb());
    assert_eq!(
        (t.opening, t.expected, t.moved_in, t.moved_out),
        (money("5000"), delta("0"), money("0"), money("0")),
        "THB: the giver keeps its own opening; 5000 − 1600 − 3400 left"
    );
    let u = row(&given, usd());
    assert_eq!(
        (u.opening, u.expected, u.moved_in, u.moved_out),
        (money("200"), delta("0"), money("0"), money("0")),
        "USD: the giver keeps its own opening; 200 + 50 − 250 left"
    );
    pool.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 row 6: order
// ─────────────────────────────────────────────────────────────────────────────

/// §9 row 6: currencies EUR, THB, USD answer THB, EUR, USD. They are opened USD first, then THB, then EUR,
/// so neither the order written nor plain A–Z gives the required order.
#[tokio::test]
async fn rows_are_thb_first_then_a_to_z() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(
        &pool,
        &ledger,
        &branch,
        &[(usd(), "5000"), (thb(), "100000"), (eur(), "5000")],
    )
    .await;
    let shift = next_shift(&pool, &branch).await;
    for (c, q) in [(usd(), "10"), (thb(), "20"), (eur(), "30")] {
        post(
            &pool,
            &ledger,
            &[transfer(&branch, &shift, c, q, true)],
            None,
            "a move into the drawer",
        )
        .await;
    }

    assert_eq!(
        currencies(&balances_of(&sb, &shift).await),
        ["THB", "EUR", "USD"]
    );
    pool.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Ruled row (Q1/Q2 on #1039): a negative drawer reads its signed value
// ─────────────────────────────────────────────────────────────────────────────

/// A transaction line for the short sale (`xc-exchange` is not a dependency of `xc-cash`, so SQL).
async fn seed_sell_line(
    pool: &PgPool,
    branch: &BranchId,
    shift: &str,
    qty: &str,
    thb_amount: &str,
) -> (TransactionLineId, String) {
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
        "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
         customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
         recorded_at, recorded_by, status, completed_at) \
         VALUES ($1, $2::uuid, $3, $4, 'LIVE', $5, $6, 'hash', $7, 'Somchai', 'THA', 'system', 'WALK-IN', \
         '', current_date, now(), now(), 'system', 'COMPLETED', now())",
    )
    .bind(tx_id)
    .bind(ORG)
    .bind(unique("DOC"))
    .bind(branch.as_str())
    .bind(shift)
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
    .bind(d(qty))
    .bind(d(thb_amount))
    .execute(pool)
    .await
    .expect("insert transaction line");
    (line_id, tx_id.to_string())
}

/// An APPROVED `NEGATIVE_STOCK` request for `object_id`, at the hash the port computes now.
async fn approved_negative_stock(
    pool: &PgPool,
    ledger: &LedgerCashPostings,
    branch: &BranchId,
    object_id: &str,
) -> ApprovalRequestId {
    let repo = PgApprovalRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    let hash = NegativeStockSource::version_hash(ledger, &mut tx, object_id)
        .await
        .expect("the source's version hash");
    let request = repo
        .insert_request(
            &mut tx,
            "NEGATIVE_STOCK",
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
    repo.update_status(&mut tx, &request.id, "APPROVED")
        .await
        .expect("approve the request");
    tx.commit().await.expect("commit");
    ApprovalRequestId::new(request.id.as_str())
}

/// Ruled row (lane controller, #1039 Q1 = B, Q2): after an approved `NEGATIVE_STOCK` sale, the drawer's
/// `ShiftBalance.expected` is the negative value, while `expected_in(Drawer(shift))` still reads 0 for it
/// (the 2026-10-10 ruling on #924 keeps `Balances` non-negative). Opening 4 USD, then SELL 10 USD for
/// 330.00 THB: USD −6.00, THB 330.00.
#[tokio::test]
async fn after_an_approved_negative_stock_sale_expected_is_negative_while_expected_in_reads_zero() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(&pool, &ledger, &branch, &[(usd(), "4")]).await;
    let shift = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &shift,
        &[(usd(), "4")],
    )
    .await;

    let (line, sale) = seed_sell_line(&pool, &branch, &shift, "10", "330.00").await;
    let approval = approved_negative_stock(&pool, &ledger, &branch, &sale).await;
    let s = src(SourceType::Exchange, &sale, "complete");
    post(
        &pool,
        &ledger,
        &[
            req(
                drawer(&shift),
                CashPlace::Customer,
                usd(),
                "10",
                &s,
                Some(line),
            ),
            req(
                CashPlace::Customer,
                drawer(&shift),
                thb(),
                "330.00",
                &s,
                None,
            ),
        ],
        Some(approval),
        "the approved short sale",
    )
    .await;

    let rows = balances_of(&sb, &shift).await;
    assert_eq!(currencies(&rows), ["THB", "USD"]);
    assert_eq!(
        row(&rows, usd()).expected,
        delta("-6"),
        "a negative drawer reads its signed value from ShiftBalances"
    );
    assert_eq!(row(&rows, usd()).opening, money("4"));
    assert_eq!(row(&rows, thb()).expected, delta("330"));

    let place = expected_in(&pool, &ledger, &shift).await;
    assert_eq!(
        place.get(usd()),
        Money::ZERO,
        "expected_in still reads 0 for the negative drawer (#924 ruling)"
    );
    assert_eq!(place.get(thb()), money("330"));
    pool.close().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// §9 row 3: the v2_shift_cases.json replay
// ─────────────────────────────────────────────────────────────────────────────

/// `Money` (and so `expected_in`) carries at most 100,000,000.00; the LG-2 parity replay leaves amounts above
/// it out of every comparison, and so does this one.
const CAP_MINOR: i64 = 10_000_000_000;

/// Foreign currencies the replay funds the branch with (the LG-2 parity replay's list).
const FUNDED: [&str; 7] = ["USD", "EUR", "GBP", "JPY", "CNY", "ZWG", "AED"];

/// How the 420 steps of `v2_shift_cases.json` map (the LG-2 parity replay's accounting,
/// `ledger_postings_test.rs` `SHIFT_*`).
const SHIFT_POSTED: usize = 218;
const SHIFT_STOCK_REFUSED: usize = 75;
const SHIFT_ONLY_SKIPPED: usize = 90;
const SHIFT_REPLAYS: usize = 8;
const SHIFT_NO_POSTING: usize = 49;
/// Shift steps where an `expected` above `CAP_MINOR` is left out (the LG-2 replay counts 4).
const CAPPED_EXPECTED: usize = 4;

fn jdec(v: &Value) -> Decimal {
    match v {
        Value::String(s) => d(s),
        Value::Number(n) => d(&n.to_string()),
        other => panic!("not an amount: {other}"),
    }
}

fn minor_of(v: Decimal) -> i64 {
    (v * Decimal::from(100))
        .round()
        .to_i64()
        .expect("fits in i64")
}

/// `(code, hundredths)` with zero and over-cap entries left out, sorted; the `bool` is true when an over-cap
/// entry was left out.
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

fn from_fixture(map: &Value) -> (Vec<(String, i64)>, bool) {
    comparable(
        map.as_object()
            .expect("an amounts object")
            .iter()
            .map(|(k, v)| (k.clone(), minor_of(jdec(v)))),
    )
}

fn i64_of(minor: i128) -> i64 {
    i64::try_from(minor).expect("fits in i64")
}

fn read_fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../xc-shift/tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed reading {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} parses: {e}"))
}

/// v2's THB effect on the place the payment method names: `round_half_up(amount × rate, 2)`, minus the fee
/// on a BUY and plus it on a SELL (the LG-2 replay's `v2_thb`).
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

/// The LG-2 replay's mapping of a v2 `tx`: the foreign leg with a line; the THB leg in `Drawer` for Cash and
/// in `Bank` otherwise; a zero THB leg is not sent.
fn v2_exchange_legs(shift: &str, t: &Value, id: &str) -> Vec<PostingRequest> {
    let s = src(SourceType::Exchange, id, "complete");
    let foreign = code(t["currency"].as_str().expect("currency"));
    let (from, to) = if t["type"] == "BUY" {
        (CashPlace::Customer, drawer(shift))
    } else {
        (drawer(shift), CashPlace::Customer)
    };
    let mut legs = vec![PostingRequest {
        from,
        to,
        currency: foreign,
        quantity: ForeignAmount::from_decimal(jdec(&t["amount"])),
        source: s.clone(),
        line: Some(Uuid::new_v4()),
        reverses: None,
    }];
    let thb_amount = v2_thb(t);
    let place = if t["method"] == "Cash" {
        drawer(shift)
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
            source: s,
            line: None,
            reverses: None,
        });
    }
    legs
}

fn v2_move(branch: &BranchId, shift: &str, payload: &Value, id: &str) -> PostingRequest {
    let s = src(SourceType::Transfer, id, "move");
    let c = code(payload["currency"].as_str().expect("currency"));
    let q = ForeignAmount::from_decimal(jdec(&payload["amount"]));
    let safe = CashPlace::Safe(branch.clone());
    let (from, to) = if payload["direction"] == "IN" {
        (safe, drawer(shift))
    } else {
        (drawer(shift), safe)
    };
    PostingRequest {
        from,
        to,
        currency: c,
        quantity: q,
        source: s,
        line: None,
        reverses: None,
    }
}

fn v2_opening_legs(
    branch: &BranchId,
    shift: &str,
    opening: &Value,
    id: &str,
) -> Vec<PostingRequest> {
    let s = src(SourceType::Opening, id, "opening");
    opening
        .as_object()
        .expect("an opening object")
        .iter()
        .map(|(c, v)| (c, jdec(v)))
        .filter(|(_, v)| !v.is_zero())
        .map(|(c, v)| PostingRequest {
            from: CashPlace::Safe(branch.clone()),
            to: drawer(shift),
            currency: code(c),
            quantity: ForeignAmount::from_decimal(v),
            source: s.clone(),
            line: None,
            reverses: None,
        })
        .collect()
}

/// The replay's branch: THB through `BANK → SAFE`, each foreign currency by a non-cash BUY into a funding
/// drawer moved on to the safe (the LG-2 replay's `fund_parity_branch`), committed.
async fn fund_replay_branch(pool: &PgPool, ledger: &LedgerCashPostings, branch: &BranchId) {
    let s = src(SourceType::Transfer, &unique("fund-safe"), "external-in");
    post(
        pool,
        ledger,
        &[
            req(
                CashPlace::Customer,
                CashPlace::Bank,
                thb(),
                "1000000000000",
                &s,
                None,
            ),
            req(
                CashPlace::Bank,
                CashPlace::Safe(branch.clone()),
                thb(),
                "1000000000000",
                &s,
                None,
            ),
        ],
        None,
        "fund THB",
    )
    .await;
    let fshift = next_shift(pool, branch).await;
    for c in FUNDED {
        let s = src(SourceType::Exchange, &format!("fund-buy-{c}"), "complete");
        post(
            pool,
            ledger,
            &[
                req(
                    CashPlace::Customer,
                    drawer(&fshift),
                    code(c),
                    "10000000000",
                    &s,
                    Some(Uuid::new_v4()),
                ),
                req(
                    CashPlace::Bank,
                    CashPlace::Customer,
                    thb(),
                    "10000000000",
                    &s,
                    None,
                ),
            ],
            None,
            "fund: BUY",
        )
        .await;
        let s = src(SourceType::Transfer, &format!("fund-move-{c}"), "to-safe");
        post(
            pool,
            ledger,
            &[req(
                drawer(&fshift),
                CashPlace::Safe(branch.clone()),
                code(c),
                "10000000000",
                &s,
                None,
            )],
            None,
            "fund: move to the safe",
        )
        .await;
    }
}

/// v2's moved figures for shift `k`: the accepted `movements` of the step's `after`, summed per currency.
fn v2_moved(movements: &Value, k: usize, direction: &str) -> (Vec<(String, i64)>, bool) {
    let mut sums: BTreeMap<String, Decimal> = BTreeMap::new();
    for m in movements.as_array().expect("movements") {
        if m["shift"].as_u64() == Some(k as u64) && m["direction"] == direction {
            *sums
                .entry(m["currency"].as_str().expect("currency").to_string())
                .or_default() += jdec(&m["amount"]);
        }
    }
    comparable(sums.into_iter().map(|(c, v)| (c, minor_of(v))))
}

/// §9 row 3: `v2_shift_cases.json` (30 scenarios) replayed through `LedgerCashPostings` with the LG-2 parity
/// replay's mapping (`open` = OPENING SAFE→DRAWER, `move` = TRANSFER SAFE↔DRAWER, `tx` = the exchange legs;
/// `count`/`close`/`abandon` post nothing; v2's stock refusals are refused by the port too). After every
/// step, for every shift so far:
/// - `expected` of every `ShiftBalance` equals `expected_in(Drawer(shift))` and the case's v2 `expected`;
/// - `opening` equals the case's v2 `opening`;
/// - `moved_in` / `moved_out` equal v2's accepted movements IN / OUT for that shift.
///
/// Every drawer here is ≥ 0, so the equality with `expected_in` holds (PR 1458).
#[tokio::test]
async fn the_v2_shift_cases_replay_gives_expected_in_and_the_v2_figures() {
    let fixture = read_fixture("v2_shift_cases.json");
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_replay_branch(&pool, &ledger, &branch).await;

    let scenarios = fixture["scenarios"].as_array().expect("scenarios");
    assert_eq!(scenarios.len(), 30, "v2_shift_cases.json has 30 scenarios");
    let (mut posted, mut stock_refused, mut shift_only, mut replays, mut no_posting) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut capped_expected = 0usize;

    for (i, sc) in scenarios.iter().enumerate() {
        let name = sc["name"].as_str().unwrap_or("");
        let mut shifts: Vec<String> = Vec::new();
        let mut seen_moves: HashSet<String> = HashSet::new();
        for (j, st) in sc["steps"].as_array().expect("steps").iter().enumerate() {
            let what = format!(
                "scenario {i} '{name}' step {j} ({})",
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
            match (st["do"].as_str().unwrap_or(""), st["v2Error"].as_str()) {
                (kind, Some(e)) if e.contains("ไม่เพียงพอ") => {
                    let reqs = match kind {
                        "move" => {
                            let s = shift_of(&st["payload"])
                                .unwrap_or_else(|| panic!("{what}: a known shift"));
                            vec![v2_move(&branch, &s, &st["payload"], &id)]
                        }
                        "tx" => v2_exchange_legs(
                            &open_shift
                                .clone()
                                .unwrap_or_else(|| panic!("{what}: an open shift")),
                            &st["tx"],
                            &id,
                        ),
                        other => panic!("{what}: a stock refusal on `{other}`"),
                    };
                    let mut tx = pool.begin().await.expect("begin");
                    match ledger.post_in(&mut tx, &reqs, None).await {
                        Err(AppError::Domain(msg)) if msg.contains("stockInsufficient") => {}
                        other => panic!("{what}: v2 refused for stock; got {other:?}"),
                    }
                    tx.rollback().await.expect("rollback");
                    stock_refused += 1;
                }
                (_, Some(_)) => shift_only += 1,
                ("open", None) => {
                    if after.len() > shifts.len() {
                        let s = next_shift(&pool, &branch).await;
                        let reqs = v2_opening_legs(&branch, &s, &st["payload"]["opening"], &id);
                        shifts.push(s);
                        if !reqs.is_empty() {
                            post(&pool, &ledger, &reqs, None, &what).await;
                        }
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
                        let s = shift_of(&st["payload"])
                            .unwrap_or_else(|| panic!("{what}: a known shift"));
                        post(
                            &pool,
                            &ledger,
                            &[v2_move(&branch, &s, &st["payload"], &id)],
                            None,
                            &what,
                        )
                        .await;
                        posted += 1;
                    } else {
                        replays += 1;
                    }
                }
                ("tx", None) => {
                    let s = open_shift.unwrap_or_else(|| panic!("{what}: an open shift"));
                    post(
                        &pool,
                        &ledger,
                        &v2_exchange_legs(&s, &st["tx"], &id),
                        None,
                        &what,
                    )
                    .await;
                    posted += 1;
                }
                ("count" | "close" | "abandon", None) => no_posting += 1,
                (other, None) => panic!("{what}: unmapped step `{other}`"),
            }
            assert_eq!(after.len(), shifts.len(), "{what}: as many shifts as v2");

            for (k, s) in shifts.iter().enumerate() {
                let at = format!("{what}, shift {k}");
                let rows = balances_of(&sb, s).await;

                let (got_expected, _) = comparable(
                    rows.iter()
                        .map(|r| (r.currency.as_str().to_string(), i64_of(r.expected.minor()))),
                );
                let (place, _) = comparable(
                    expected_in(&pool, &ledger, s)
                        .await
                        .non_zero()
                        .into_iter()
                        .map(|(c, m)| (c.as_str().to_string(), m.try_minor().unwrap())),
                );
                let (want_expected, dropped) = from_fixture(&after[k]["expected"]);
                capped_expected += usize::from(dropped);
                assert_eq!(
                    got_expected, place,
                    "{at}: expected = expected_in(Drawer(shift))"
                );
                assert_eq!(
                    got_expected, want_expected,
                    "{at}: expected = v2's expected"
                );

                let (got_opening, _) = comparable(rows.iter().map(|r| {
                    (
                        r.currency.as_str().to_string(),
                        r.opening.try_minor().unwrap(),
                    )
                }));
                assert_eq!(
                    got_opening,
                    from_fixture(&after[k]["opening"]).0,
                    "{at}: opening = v2's opening"
                );

                for (direction, pick) in [("IN", true), ("OUT", false)] {
                    let (got, _) = comparable(rows.iter().map(|r| {
                        let m = if pick { r.moved_in } else { r.moved_out };
                        (r.currency.as_str().to_string(), m.try_minor().unwrap())
                    }));
                    assert_eq!(
                        got,
                        v2_moved(&st["after"]["movements"], k, direction).0,
                        "{at}: moved {direction} = v2's accepted movements {direction}"
                    );
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
        "every step is accounted for (posted, stock refusals, shift-only refusals, replays, close/abandon)"
    );
    assert_eq!(
        capped_expected, CAPPED_EXPECTED,
        "comparisons with an expected above 100,000,000 left out"
    );
    pool.close().await;
}

/// Admin shifts reads every listed shift's balances in one call (`ShiftBalances::shift_balances_many`,
/// BE controller claude/7ca80c; up-front test by claude/d7aa64). The batched read answers, for every
/// requested shift, exactly what `shift_balances` answers for it alone, and every requested id is a key
/// (an empty list when the shift has no rows).
#[tokio::test]
async fn shift_balances_many_answers_each_shifts_own_rows_like_shift_balances() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(
        &pool,
        &ledger,
        &branch,
        &[(thb(), "100000"), (usd(), "5000"), (eur(), "3000")],
    )
    .await;

    let s1 = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &s1,
        &[(thb(), "10000"), (usd(), "1000")],
    )
    .await;
    post(
        &pool,
        &ledger,
        &exchange(&s1, "BUY", usd(), "100", "3200", &unique("ex")),
        None,
        "BUY",
    )
    .await;

    let s2 = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &s2,
        &[(thb(), "2000"), (eur(), "500")],
    )
    .await;

    let s3 = next_shift(&pool, &branch).await; // no posting, no count

    let ids = vec![s1.clone(), s2.clone(), s3.clone()];
    let many = sb
        .shift_balances_many(&ids)
        .await
        .expect("the batched read");

    let keys: Vec<&String> = many.keys().collect();
    let mut want: Vec<&String> = ids.iter().collect();
    want.sort();
    assert_eq!(keys, want, "every requested shift is a key");
    for id in &ids {
        assert_eq!(
            many[id],
            balances_of(&sb, id).await,
            "{id}: the batched rows equal the shift's own read"
        );
    }
    assert!(
        many[&s3].is_empty(),
        "a shift with no rows answers an empty list"
    );
    pool.close().await;
}

/// Two CLOSED shifts that both counted THB, with different verified CLOSE counts: the batched read
/// must give each shift its own `counted`/`difference`, never the other shift's.
#[tokio::test]
async fn shift_balances_many_keeps_each_closed_shifts_own_count() {
    let (pool, branch) = db().await;
    let ledger = LedgerCashPostings::new();
    let sb = PgLedgerShiftBalances::new(pool.clone());
    fund_safe(&pool, &ledger, &branch, &[(thb(), "100000")]).await;

    let a = next_shift(&pool, &branch).await;
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &a,
        &[(thb(), "10000")],
    )
    .await;
    seed_count(
        &pool,
        &branch,
        &a,
        "CLOSE",
        true,
        &[(thb(), "9990", "10000", "-10")],
    )
    .await;

    let b = next_shift(&pool, &branch).await; // closes a
    open_from(
        &pool,
        &ledger,
        CashPlace::Safe(branch.clone()),
        &b,
        &[(thb(), "20000")],
    )
    .await;
    seed_count(
        &pool,
        &branch,
        &b,
        "CLOSE",
        true,
        &[(thb(), "20050", "20000", "50")],
    )
    .await;
    let _c = next_shift(&pool, &branch).await; // closes b

    let ids = vec![a.clone(), b.clone()];
    let many = sb
        .shift_balances_many(&ids)
        .await
        .expect("the batched read");
    for id in &ids {
        assert_eq!(
            many[id],
            balances_of(&sb, id).await,
            "{id}: batched rows equal the shift's own read"
        );
    }
    assert_eq!(
        row(&many[&a], thb()).counted,
        Some(money("9990")),
        "A's own count"
    );
    assert_eq!(
        row(&many[&b], thb()).counted,
        Some(money("20050")),
        "B's own count"
    );
    pool.close().await;
}
