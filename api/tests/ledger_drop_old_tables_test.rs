//! Ticket #1047 (LG-6.m) — migration 3: `api/migrations/<version>_drop_shift_balances.sql` drops `shift_balances`,
//! `stock_movements` and `ops_state.safe` behind a reconcile precheck (design
//! `docs/design/blueprint/ledger-inventory.md` §9 migration 3, §6.1; ticket §3, §4 criterion 2, §10).
//! Written before the implementation by the ticket author's delegate (claude, for the lane controller). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! What this test pins ([required]):
//! - **The file.** Exactly one `api/migrations/*_drop_shift_balances.sql`, sorting after
//!   `20261218110000_retire_custody_requests.sql` (the newest on `feat/lg6` when this was written; the version is
//!   picked above main's newest at push time, which `migration-order.sh` enforces). No reversible (`.up.`/`.down.`)
//!   file. It carries a `-- down:` section, read as the LG-4 test reads it: the lines after the line `-- down:`, each
//!   with its leading `--` and at most one following space removed, run as one batch.
//! - **Clean ledger ⇒ the stores go.** On a ledger that reconciles (an opening, a BUY, a SELL costed through the
//!   ledger, a drawer → safe move), the migration applies; afterwards `shift_balances` and `stock_movements` do not
//!   exist and `ops_state` has every column it had except `safe`, with its rows kept.
//! - **The tie to LG-7's `QUERY` constants** (ticket §4 criterion 2): five seeds, seed *k* hand-breaking one row that
//!   only check *k* catches. On each, the test first asserts that `Reconcile::standard().run` fails exactly check *k*
//!   (so a seed that leaks into another check is a test defect, not drift). Then the migration is refused with an
//!   error whose text contains `reconcile check <k> failed`, is not recorded as applied, and drops nothing (one
//!   transaction): both tables, their rows and `ops_state.safe` are still there.
//! - **`down`** after a successful up re-creates the three stores, empty, with the columns they had before the up.
//!
//! Hand-broken rows are written with triggers off (`session_replication_role = replica`), as the LG-7 tests
//! (`api/crates/xc-cash/tests/reconcile_*_test.rs`) store them.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Each test has its own schema (`quote_common::TestDb`), migrated up to
//! the migration before this one; the ledger is seeded through `LedgerCashPostings`, then the migration under test (and
//! nothing later) is run.

mod common;
mod quote_common;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use quote_common::TestDb;
use rust_decimal::Decimal;
use sqlx::AssertSqlSafe;
use sqlx::migrate::Migrator;
use uuid::Uuid;
use xc_cash::application::reconcile::{Reconcile, ReconcileScope};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

/// The newest migration on `feat/lg6` when this was written.
const BEFORE: &str = "20261218110000_retire_custody_requests.sql";
const SUFFIX: &str = "_drop_shift_balances.sql";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const SHIFT: &str = "SH-LG6M";

/// Tables today's `LedgerCashPostings` reads that are created by migrations after the drop. The seed runs current code
/// against the pre-drop schema, so each such table must exist first. When `post_in` learns to read a newer table, add
/// its migration here.
const SEED_SCHEMA_EXTRAS: &[&str] = &["stock_reservations"];

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

fn names() -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(migrations())
        .expect("read api/migrations")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .filter(|n| n.ends_with(".sql"))
        .collect();
    n.sort();
    n
}

/// (version, file name, text) of the one drop migration.
fn the_file() -> (i64, String, String) {
    let found: Vec<String> = names()
        .into_iter()
        .filter(|n| n.ends_with(SUFFIX))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/*{SUFFIX} (design §9 migration 3, ticket #1047 §3): {found:?}"
    );
    let name = found[0].clone();
    let version: i64 = name[..name.find('_').expect("<version>_")]
        .parse()
        .expect("a numeric version");
    let text = std::fs::read_to_string(migrations().join(&name)).expect("read the migration");
    (version, name, text)
}

/// The largest migration version below `v`.
fn version_before(v: i64) -> i64 {
    names()
        .iter()
        .filter_map(|n| n.split('_').next().and_then(|x| x.parse::<i64>().ok()))
        .filter(|x| *x < v)
        .max()
        .expect("a migration before it")
}

/// The `-- down:` section, uncommented.
fn down_sql(text: &str) -> String {
    let mut lines = text.lines();
    assert!(
        lines.any(|l| l.trim() == "-- down:"),
        "the file carries a `-- down:` line"
    );
    let mut out = String::new();
    for l in lines {
        let t = l.trim_start();
        let body = t.strip_prefix("--").unwrap_or(t);
        let body = body.strip_prefix(' ').unwrap_or(body);
        out.push_str(body);
        out.push('\n');
    }
    assert!(!out.trim().is_empty(), "the down section is not empty");
    out
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").expect("USD")
}

// ── the seeded world ────────────────────────────────────────────────────────

/// One request of a posting.
fn req(
    from: CashPlace,
    to: CashPlace,
    currency: CurrencyCode,
    quantity: &str,
    source: &SourceRef,
    line: Option<Uuid>,
) -> PostingRequest {
    PostingRequest {
        from,
        to,
        currency,
        quantity: ForeignAmount::from_decimal(d(quantity)),
        source: source.clone(),
        line,
        reverses: None,
    }
}

async fn post(db: &TestDb, requests: &[PostingRequest]) {
    let mut tx = db.pool.begin().await.expect("begin");
    LedgerCashPostings::new()
        .post_in(&mut tx, requests, None)
        .await
        .unwrap_or_else(|e| panic!("post {:?}: {e:?}", requests[0].source));
    tx.commit().await.expect("commit the posting");
}

fn source(source_type: SourceType, id: &str, key: &str) -> SourceRef {
    SourceRef {
        source_type,
        source_id: id.to_owned(),
        source_key: key.to_owned(),
    }
}

/// A COMPLETED transaction at MAIN with one SELL line (USD 40 for THB 1300.00), not yet costed. Answers the line id.
async fn sell_transaction(db: &TestDb) -> Uuid {
    let tx_id = Uuid::new_v4();
    let line_id = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
             customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
             recorded_by, completed_at, status, organization_id) \
         VALUES ('{tx_id}', 'TX-LG6M-1', 'MAIN', 'LIVE', '{SHIFT}', 'sub-lg6m-1', 'hash', 'CUST-LG6M', 'Customer', \
             'THA', 'staff', '', '', current_date, now(), now(), 'staff', now(), 'COMPLETED', '{ORG}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
             denomination_id, quantity, unit_size, rate_snapshot, reference_rate, thb_amount, fee_thb, settle_thb, \
             cost_basis_thb, realized_margin_thb, line_no, created_at, updated_at) \
         VALUES ('{line_id}', '{ORG}', '{tx_id}', NULL, 'SELL', 'USD', NULL, 40.0, 1.0, 32.5, NULL, 1300.00, 0.00, \
             1300.00, NULL, NULL, 1, now(), now())"
    ))
    .await;
    line_id
}

/// A clean ledger at MAIN (one shift, `SHIFT`):
/// - OPENING: THB 10000 into the drawer;
/// - EXCHANGE (BUY): USD 100 in from the customer for THB 3210 out (a lot at 32.10);
/// - EXCHANGE (SELL): USD 40 out for THB 1300 in, on a stored SELL line, costed by the ledger (cost 1284, margin 16);
/// - TRANSFER: USD 30 drawer → safe.
///
/// Plus the v2 rows this migration drops: one `shift_balances` row, one `stock_movements` row and `ops_state.safe`.
async fn clean_ledger(db: &TestDb) {
    db.customer("CUST-LG6M", "MAIN").await;
    db.exec(&format!(
        "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ('{ORG}', 'Teller LG6M', 'ACTIVE', 'test') RETURNING id) \
         INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         SELECT '{SHIFT}', 'MAIN', 'OPEN', now(), 'owner', 'open-{SHIFT}', u.id FROM u"
    ))
    .await;

    let drawer = || CashPlace::Drawer(SHIFT.into());
    let opening = source(SourceType::Opening, "lg6m-opening", "open");
    post(
        db,
        &[req(
            CashPlace::Customer,
            drawer(),
            CurrencyCode::THB,
            "10000.00",
            &opening,
            None,
        )],
    )
    .await;

    let buy = source(SourceType::Exchange, "lg6m-buy", "exchange");
    post(
        db,
        &[
            req(CashPlace::Customer, drawer(), usd(), "100", &buy, None),
            req(
                drawer(),
                CashPlace::Customer,
                CurrencyCode::THB,
                "3210.00",
                &buy,
                None,
            ),
        ],
    )
    .await;

    let line = sell_transaction(db).await;
    let sell = source(SourceType::Exchange, "lg6m-sell", "exchange");
    post(
        db,
        &[
            req(
                drawer(),
                CashPlace::Customer,
                usd(),
                "40",
                &sell,
                Some(line),
            ),
            req(
                CashPlace::Customer,
                drawer(),
                CurrencyCode::THB,
                "1300.00",
                &sell,
                None,
            ),
        ],
    )
    .await;

    let to_safe = source(SourceType::Transfer, "lg6m-move", "to-safe");
    post(
        db,
        &[req(
            drawer(),
            CashPlace::Safe(BranchId::new("MAIN")),
            usd(),
            "30",
            &to_safe,
            None,
        )],
    )
    .await;

    db.exec(&format!(
        "INSERT INTO shift_balances (shift_id, code, opening, expected) VALUES ('{SHIFT}', 'USD', 0, 30)"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO stock_movements (id, branch_id, shift_id, code, direction, amount, reason, request_id, at, by) \
         VALUES ('SM-LG6M', 'MAIN', '{SHIFT}', 'USD', 'OUT', 30, 'to safe', 'req-lg6m', now(), 'owner')"
    ))
    .await;
    db.exec(
        "INSERT INTO ops_state (branch_id, phase, safe) VALUES ('MAIN', 'OPEN', '{\"USD\": 3000}'::jsonb) \
         ON CONFLICT (branch_id) DO UPDATE SET safe = EXCLUDED.safe",
    )
    .await;

    // The seed is a real ledger, not an empty one: every check has rows to read.
    assert_eq!(
        db.count("SELECT count(*) FROM ledger_postings").await,
        4,
        "four postings"
    );
    assert!(
        db.count("SELECT count(*) FROM inventory_movements").await >= 2,
        "the BUY and the SELL moved inventory"
    );
    assert_eq!(
        db.count(
            "SELECT count(*) FROM transaction_lines \
             WHERE shop_side = 'SELL' AND cost_basis_thb IS NOT NULL AND realized_margin_thb IS NOT NULL"
        )
        .await,
        1,
        "the SELL line was costed through the ledger"
    );
}

/// Runs `sql` with triggers off, as a hand-edited row would be stored.
async fn hand_edit(db: &TestDb, sql: &str) {
    let mut tx = db.pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .expect("triggers off for this transaction");
    let done = sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    assert!(
        done.rows_affected() >= 1,
        "the hand edit changed a row: {sql}"
    );
    tx.commit().await.expect("commit the hand edit");
}

/// Seed *k* of ticket §4 criterion 2: one hand-broken row only check *k* catches.
async fn break_check(db: &TestDb, k: u8) {
    let customer_usd_entry = "(SELECT e.id FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
         WHERE a.kind = 'CUSTOMER' AND a.currency = 'USD' ORDER BY e.id LIMIT 1)";
    match k {
        // a CUSTOMER-account entry's quantity (foreign currency) +1; its valuation untouched
        1 => {
            hand_edit(
                db,
                &format!(
                    "UPDATE ledger_entries SET quantity_signed = quantity_signed + 1 WHERE id = {customer_usd_entry}"
                ),
            )
            .await
        }
        // one movement's cost +0.01; the position untouched
        2 => {
            hand_edit(
                db,
                "UPDATE inventory_movements SET cost_thb_signed = cost_thb_signed + 0.01 \
                 WHERE id = (SELECT min(id) FROM inventory_movements)",
            )
            .await
        }
        // a balanced SAFE ↔ CUSTOMER pair (quantities and valuations net zero), with no inventory movement
        3 => {
            hand_edit(
                db,
                &format!(
                    "WITH p AS ( \
                         INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, \
                             source_key, status, posted_at, posted_by) \
                         VALUES (gen_random_uuid(), '{ORG}', 'MAIN', 'ADJUSTMENT', 'lg6m-seed3', 'k', 'POSTED', \
                             now(), 'test') \
                         RETURNING id) \
                     INSERT INTO ledger_entries (posting_id, account_id, currency, quantity_signed, valuation_thb, \
                         created_at) \
                     SELECT p.id, a.id, 'USD', CASE a.kind WHEN 'SAFE' THEN 10 ELSE -10 END, \
                            CASE a.kind WHEN 'SAFE' THEN 321.00 ELSE -321.00 END, now() \
                     FROM p, cash_accounts a \
                     WHERE a.currency = 'USD' AND ((a.kind = 'SAFE' AND a.branch_id = 'MAIN') OR a.kind = 'CUSTOMER')"
                ),
            )
            .await;
            assert_eq!(
                db.count(
                    "SELECT count(*) FROM ledger_entries e JOIN ledger_postings p ON p.id = e.posting_id \
                     WHERE p.source_id = 'lg6m-seed3'"
                )
                .await,
                2,
                "seed 3 is one pair: one SAFE and one CUSTOMER entry"
            );
        }
        // one SELL line's realized margin +1.00
        4 => {
            hand_edit(
                db,
                "UPDATE transaction_lines SET realized_margin_thb = realized_margin_thb + 1.00 \
                 WHERE shop_side = 'SELL'",
            )
            .await
        }
        // one entry's valuation +0.01; quantities untouched
        5 => {
            hand_edit(
                db,
                &format!(
                    "UPDATE ledger_entries SET valuation_thb = valuation_thb + 0.01 WHERE id = {customer_usd_entry}"
                ),
            )
            .await
        }
        _ => unreachable!("checks are 1..=5"),
    }
}

fn extra_up_sql(extra: &str) -> String {
    let suffix = format!("_{extra}.sql");
    let found: Vec<String> = names()
        .into_iter()
        .filter(|n| n.ends_with(&suffix))
        .collect();
    let [name] = found.as_slice() else {
        panic!("expected one api/migrations/*{suffix}: {found:?}");
    };
    let text = std::fs::read_to_string(migrations().join(name)).expect("read the extra migration");
    text.lines()
        .take_while(|l| l.trim() != "-- down:")
        .collect::<Vec<_>>()
        .join("\n")
}

/// A schema migrated up to the migration before the drop, with the clean ledger and, for `Some(k)`, seed *k*.
async fn seeded(k: Option<u8>) -> (TestDb, i64) {
    let (version, _, _) = the_file();
    let db = TestDb::empty().await;
    db.migrate_through(Some(version_before(version))).await;
    for extra in SEED_SCHEMA_EXTRAS {
        sqlx::raw_sql(AssertSqlSafe(extra_up_sql(extra)))
            .execute(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("seed extra {extra}: {e}"));
    }
    clean_ledger(&db).await;
    if let Some(k) = k {
        break_check(&db, k).await;
    }
    (db, version)
}

/// The migrations after the seed, up to and including the drop and nothing later, answering the error text when one
/// fails.
async fn run_the_rest(db: &TestDb) -> Result<(), String> {
    let (version, _, _) = the_file();
    let dir = migrations();
    let mut migrator = Migrator::new(dir.as_path())
        .await
        .expect("read api/migrations");
    let through: Vec<_> = migrator
        .migrations
        .iter()
        .filter(|m| m.version <= version)
        .cloned()
        .collect();
    migrator.migrations = std::borrow::Cow::Owned(through);
    migrator
        .run(&db.pool)
        .await
        .map_err(|e| format!("{e:?} / {e}"))
}

async fn run_down(db: &TestDb) -> Result<(), String> {
    let (_, _, text) = the_file();
    sqlx::raw_sql(AssertSqlSafe(down_sql(&text)))
        .execute(&db.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The ids of the checks `Reconcile::standard().run` reports failed for the organisation (no branch, no period).
async fn failed_checks(db: &TestDb) -> Vec<u8> {
    let report = Reconcile::standard()
        .run(
            &db.pool,
            &ReconcileScope {
                organization_id: ORG.to_owned(),
                branch: None,
                period: None,
            },
        )
        .await
        .expect("reconcile runs");
    let mut ids: Vec<u8> = report.checks.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![1, 2, 3, 4, 5],
        "the five checks of design §6 are registered"
    );
    report
        .checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| c.id)
        .collect()
}

async fn table_exists(db: &TestDb, table: &str) -> bool {
    db.count(&format!(
        "SELECT count(*) FROM information_schema.tables \
         WHERE table_schema = current_schema() AND table_name = '{table}'"
    ))
    .await
        > 0
}

/// A table's column names, sorted.
async fn columns(db: &TestDb, table: &str) -> Vec<String> {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = '{table}' ORDER BY 1"
    )))
    .fetch_all(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("columns of {table}: {e}"))
}

/// `ops_state` rows without `safe`, as text.
async fn ops_state_without_safe(db: &TestDb) -> Vec<String> {
    sqlx::query_scalar("SELECT (to_jsonb(o) - 'safe')::text FROM ops_state o ORDER BY branch_id")
        .fetch_all(&db.pool)
        .await
        .expect("read ops_state")
}

async fn applied(db: &TestDb, version: i64) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM _sqlx_migrations WHERE version = {version} AND success"
    ))
    .await
}

// ── the file ────────────────────────────────────────────────────────────────

#[test]
fn the_file_sorts_after_the_newest_migration_and_carries_a_down_section() {
    let (_, name, text) = the_file();
    let all = names();
    assert!(all.iter().any(|n| n == BEFORE), "{BEFORE} is on the branch");
    assert!(name.as_str() > BEFORE, "{name} sorts after {BEFORE}");
    assert!(
        all.iter()
            .all(|n| !n.contains(".up.") && !n.contains(".down.")),
        "no reversible migration: {all:?}"
    );
    down_sql(&text);
}

// ── clean ledger ────────────────────────────────────────────────────────────

/// Ticket §7–9 outcome 1: a database where the ledger reconciles loses the three stores; `ops_state` keeps its
/// other columns and its rows.
#[tokio::test]
async fn a_clean_ledger_lets_the_migration_drop_the_three_stores() {
    let (db, version) = seeded(None).await;
    assert_eq!(
        failed_checks(&db).await,
        Vec::<u8>::new(),
        "the clean seed reconciles"
    );
    let ops_columns = columns(&db, "ops_state").await;
    assert!(
        ops_columns.iter().any(|c| c == "safe"),
        "ops_state.safe exists before the drop"
    );
    let ops_rows = ops_state_without_safe(&db).await;

    run_the_rest(&db)
        .await
        .expect("the drop applies on a clean ledger");

    assert_eq!(applied(&db, version).await, 1, "recorded as applied");
    assert!(
        !table_exists(&db, "shift_balances").await,
        "shift_balances is dropped"
    );
    assert!(
        !table_exists(&db, "stock_movements").await,
        "stock_movements is dropped"
    );
    assert_eq!(
        columns(&db, "ops_state").await,
        ops_columns
            .into_iter()
            .filter(|c| c != "safe")
            .collect::<Vec<_>>(),
        "ops_state loses only `safe`"
    );
    assert_eq!(
        ops_state_without_safe(&db).await,
        ops_rows,
        "every ops_state row is kept, its other columns unchanged"
    );
    assert_eq!(
        failed_checks(&db).await,
        Vec::<u8>::new(),
        "the ledger is untouched and still reconciles"
    );
    db.finish().await;
}

// ── the tie to the QUERY constants ──────────────────────────────────────────

/// Seed `k`: `run` fails exactly check `k`; the migration is refused naming check `k`; nothing is dropped.
async fn seed_k_is_refused_naming_check_k(k: u8) {
    let (db, version) = seeded(Some(k)).await;
    assert_eq!(
        failed_checks(&db).await,
        vec![k],
        "seed {k} breaks check {k} and only check {k} (a leak is a defect of this test)"
    );
    let before = (
        db.count("SELECT count(*) FROM shift_balances").await,
        db.count("SELECT count(*) FROM stock_movements").await,
        db.text("SELECT safe::text FROM ops_state WHERE branch_id = 'MAIN'")
            .await,
    );

    let err = run_the_rest(&db)
        .await
        .expect_err("a ledger that does not reconcile refuses the drop");
    let want = format!("reconcile check {k} failed");
    assert!(
        err.contains(&want),
        "the error names check {k} ({want:?}): {err}"
    );
    for other in (1u8..=5).filter(|j| *j != k) {
        assert!(
            !err.contains(&format!("reconcile check {other} failed")),
            "the error names only check {k}: {err}"
        );
    }

    assert_eq!(applied(&db, version).await, 0, "not recorded as applied");
    assert!(
        table_exists(&db, "shift_balances").await,
        "shift_balances is still there"
    );
    assert!(
        table_exists(&db, "stock_movements").await,
        "stock_movements is still there"
    );
    assert!(
        columns(&db, "ops_state").await.iter().any(|c| c == "safe"),
        "ops_state.safe is still there"
    );
    assert_eq!(
        (
            db.count("SELECT count(*) FROM shift_balances").await,
            db.count("SELECT count(*) FROM stock_movements").await,
            db.text("SELECT safe::text FROM ops_state WHERE branch_id = 'MAIN'")
                .await,
        ),
        before,
        "nothing is dropped or deleted (one transaction)"
    );
    db.finish().await;
}

/// Seed 1: a CUSTOMER entry's quantity +1 ⇒ check 1 (every posting balances per currency).
#[tokio::test]
async fn seed_1_an_unbalanced_customer_entry_refuses_the_drop_naming_check_1() {
    seed_k_is_refused_naming_check_k(1).await;
}

/// Seed 2: one movement's cost +0.01 ⇒ check 2 (a position equals Σ its movements).
#[tokio::test]
async fn seed_2_a_movement_cost_off_by_a_satang_refuses_the_drop_naming_check_2() {
    seed_k_is_refused_naming_check_k(2).await;
}

/// Seed 3: a balanced SAFE ↔ CUSTOMER pair with no movement ⇒ check 3 (places equal the position).
#[tokio::test]
async fn seed_3_a_safe_customer_pair_without_a_movement_refuses_the_drop_naming_check_3() {
    seed_k_is_refused_naming_check_k(3).await;
}

/// Seed 4: one SELL line's margin +1.00 ⇒ check 4 (margin = sales − cost).
#[tokio::test]
async fn seed_4_a_sell_margin_off_by_one_baht_refuses_the_drop_naming_check_4() {
    seed_k_is_refused_naming_check_k(4).await;
}

/// Seed 5: one entry's valuation +0.01 ⇒ check 5 (Σ valuation per posting is 0).
#[tokio::test]
async fn seed_5_a_valuation_off_by_a_satang_refuses_the_drop_naming_check_5() {
    seed_k_is_refused_naming_check_k(5).await;
}

// ── down ────────────────────────────────────────────────────────────────────

/// §10 item 4: `down` re-creates the three stores, empty, with the columns they had; the old figures are not
/// restored.
#[tokio::test]
async fn down_re_creates_the_three_stores_empty() {
    let (db, _) = seeded(None).await;
    let shape = (
        columns(&db, "shift_balances").await,
        columns(&db, "stock_movements").await,
        columns(&db, "ops_state").await,
    );
    run_the_rest(&db).await.expect("the drop applies");
    run_down(&db).await.expect("down runs after the drop");

    assert!(
        table_exists(&db, "shift_balances").await,
        "shift_balances is back"
    );
    assert!(
        table_exists(&db, "stock_movements").await,
        "stock_movements is back"
    );
    assert_eq!(
        (
            columns(&db, "shift_balances").await,
            columns(&db, "stock_movements").await,
            columns(&db, "ops_state").await,
        ),
        shape,
        "the three stores have the columns they had before the drop"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM shift_balances").await,
        0,
        "shift_balances is empty"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM stock_movements").await,
        0,
        "stock_movements is empty"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM ops_state WHERE safe IS NOT NULL")
            .await,
        0,
        "ops_state.safe is back, empty"
    );
    db.finish().await;
}
