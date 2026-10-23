//! Ticket #929 (LG-4) — migration 2, the opening backfill: `api/migrations/<version>_ledger_opening_balances.sql`
//! (design `docs/design/blueprint/ledger-inventory.md` §9 item 2; ticket §8, §9, §10; owner's Q-3, 2026-10-10:
//! the opening cost is the branch's published BUY rate).
//! Written before the implementation by the ticket author (claude/89bd61d9, rule A). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! What this test pins ([required] unless marked):
//! - **The file.** Exactly one `api/migrations/*_ledger_opening_balances.sql`, sorting after
//!   `20261112110000_reversals.sql` (main's newest when this was written; the lane controller's rule: above main's
//!   newest at push time, which `migration-order.sh` enforces). No reversible (`.up.`/`.down.`) file.
//! - **End state, not posting shape** (ticket author's pin, accepted by the lane controller on #929): after the
//!   backfill, read through the port (`LedgerCashPostings::expected_in`):
//!   - every OPEN shift's `Drawer(shift)` equals that shift's `shift_balances.expected`, currency by currency;
//!   - every branch's `Safe(branch)` equals `ops_state.safe` (`{"<CUR>": <minor units>}`, the custody codec);
//!   - a CLOSED shift's drawer holds nothing;
//!   - these are the figures the interim port answered before the cutover (Drawer = `shift_balances.expected`,
//!     Safe = `ops_state.safe`), so the cutover changes no balance anyone sees.
//! - Every posting the backfill writes has `source_type = 'OPENING'`.
//! - **Inventory (Q-3):** for every branch and foreign currency with a positive balance (its safe plus its OPEN
//!   drawers), lots whose quantities sum to that balance, each at `unit_cost_thb` = the branch's current published
//!   BUY rate ÷ `unit_size` (8 places, half away from zero), and one `inventory_positions` row with that quantity and
//!   `cost_thb` = Σ its lots' quantity × unit cost (2 places, half away from zero). THB is not inventory: no THB lot
//!   or position.
//! - **Reconciliation (§6) is clean** after the backfill: checks 1, 2, 3 and 5 report no offender.
//! - **No BUY rate:** a foreign currency with a positive balance and no published BUY rate at its branch stops the
//!   migration with an error whose text names the currency, and nothing is written (a silent zero cost would be
//!   wrong).
//! - **v2 rows are read, not changed:** `shift_balances`, `stock_movements` and `ops_state` are byte-equal before and
//!   after.
//! - **`down`** is the file's `-- down:` section (as every v3 migration carries one): the lines after the line
//!   `-- down:`, each with its leading `--` and at most one following space removed, run as one batch. It is
//!   refused (an error, nothing deleted) once any posting other than `OPENING` exists; otherwise it deletes the
//!   `OPENING` postings' movements and entries, their lots, the postings, and every position.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Each test has its own schema (`quote_common::TestDb`). The v2 rows
//! are seeded with SQL before migration 2 runs, as `rates_re4e_readers_test.rs` seeds before its conversion.

mod common;
mod quote_common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use quote_common::{Line, TestDb};
use rust_decimal::{Decimal, RoundingStrategy};
use sqlx::AssertSqlSafe;
use sqlx::migrate::Migrator;
use xc_cash::application::reconcile::{Reconcile, ReconcileScope};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

/// Main's newest migration when this was written.
const BEFORE: &str = "20261112110000_reversals.sql";
const SUFFIX: &str = "_ledger_opening_balances.sql";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

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

/// (version, file name, text) of the one opening-balances migration.
fn the_file() -> (i64, String, String) {
    let found: Vec<String> = names()
        .into_iter()
        .filter(|n| n.ends_with(SUFFIX))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/*{SUFFIX} (design §9 item 2): {found:?}"
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
    s.parse().unwrap()
}

fn cur(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).unwrap()
}

// ── the v2 world before the cutover ─────────────────────────────────────────

/// Branch MAIN (the migrations seed it), B2 and B3. MAIN publishes USD BUY 32.10 / 1 and JPY BUY 23.5 / 100; B2
/// publishes USD BUY 32.40 / 1; B3 publishes nothing and holds only THB (THB needs no rate). OPEN shifts SH-M1 (MAIN),
/// SH-M2 (B3, THB only, a zero USD row) and SH-B1 (B2); SH-MC is CLOSED at MAIN (its balances are history). MAIN's
/// and B2's safes are in `ops_state.safe`. (v2 allows one v2-opened OPEN shift per branch, so each OPEN shift here
/// is at its own branch.)
async fn seed_v2(db: &TestDb, gbp_in_main_safe: bool) {
    db.branch("B2").await;
    db.branch("B3").await;
    db.publish(
        "MAIN",
        &[
            Line::manual("USD", "1", "32.10", "32.45"),
            Line::manual("JPY", "100", "23.5", "24.0"),
        ],
    )
    .await;
    db.publish("B2", &[Line::manual("USD", "1", "32.40", "32.80")])
        .await;
    for (id, branch, status) in [
        ("SH-M1", "MAIN", "OPEN"),
        ("SH-M2", "B3", "OPEN"),
        ("SH-MC", "MAIN", "CLOSED"),
        ("SH-B1", "B2", "OPEN"),
    ] {
        let closed = if status == "CLOSED" { "now()" } else { "NULL" };
        // one teller per shift (CA-1's `shifts.teller_id`)
        db.exec(&format!(
            "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                 SELECT organization_id, 'Teller {id}', 'ACTIVE', 'test' FROM branches WHERE id = '{branch}' \
                 RETURNING id) \
             INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, closed_at, teller_id) \
             SELECT '{id}', '{branch}', '{status}', now(), 'owner', 'open-{id}', {closed}, u.id FROM u"
        ))
        .await;
    }
    for (shift, code, expected) in [
        ("SH-M1", "THB", "10000.00"),
        ("SH-M1", "USD", "500.00"),
        ("SH-M1", "JPY", "20000.00"),
        ("SH-M2", "THB", "5000.00"),
        ("SH-M2", "USD", "0.00"),
        ("SH-MC", "USD", "999.00"),
        ("SH-B1", "USD", "100.00"),
    ] {
        db.exec(&format!(
            "INSERT INTO shift_balances (shift_id, code, opening, expected) \
             VALUES ('{shift}', '{code}', {expected}, {expected})"
        ))
        .await;
    }
    let main_safe = if gbp_in_main_safe {
        "'{\"THB\": 5000000, \"USD\": 200000, \"JPY\": 1000000, \"GBP\": 5000}'"
    } else {
        "'{\"THB\": 5000000, \"USD\": 200000, \"JPY\": 1000000}'"
    };
    db.exec(&format!(
        "INSERT INTO ops_state (branch_id, enabled, phase, safe) VALUES \
         ('MAIN', true, 'OPEN', {main_safe}::jsonb), ('B2', true, 'OPEN', '{{\"USD\": 30000}}'::jsonb)"
    ))
    .await;
}

/// A schema migrated up to the migration before the opening backfill, with the v2 world seeded.
async fn before_the_backfill(gbp_in_main_safe: bool) -> (TestDb, i64) {
    let (version, _, _) = the_file();
    let db = TestDb::empty().await;
    db.migrate_through(Some(version_before(version))).await;
    seed_v2(&db, gbp_in_main_safe).await;
    (db, version)
}

/// The migrations after the seed, up to and including migration 2 itself and nothing later, answering the error
/// text when one fails. Bounded (rule-A review gap 3, ruled issuecomment-6115022197): a later migration that changes
/// or drops the v2 tables these tests seed and read (§6) must not reach them.
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

async fn backfilled() -> TestDb {
    let (db, _) = before_the_backfill(false).await;
    run_the_rest(&db)
        .await
        .expect("migration 2 applies on the seeded v2 world");
    db
}

/// A place's balances through the port, as `{currency: decimal}` without zeros.
async fn balances(db: &TestDb, place: CashPlace) -> BTreeMap<String, Decimal> {
    let mut tx = db.pool.begin().await.unwrap();
    let b = LedgerCashPostings::new()
        .expected_in(&mut tx, &place)
        .await
        .unwrap_or_else(|e| panic!("expected_in {place:?}: {e:?}"));
    tx.rollback().await.unwrap();
    b.non_zero()
        .into_iter()
        .map(|(c, m)| (c.as_str().to_owned(), m.value().round_dp(2)))
        .collect()
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, Decimal> {
    pairs.iter().map(|(c, v)| ((*c).to_owned(), d(v))).collect()
}

async fn snapshot(db: &TestDb, table: &str) -> Vec<String> {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT row_to_json(t)::text FROM {table} t ORDER BY row_to_json(t)::text"
    )))
    .fetch_all(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("snapshot {table}: {e}"))
}

async fn count(db: &TestDb, table: &str) -> i64 {
    db.count(&format!("SELECT count(*) FROM {table}")).await
}

// ── the file ────────────────────────────────────────────────────────────────

#[test]
fn the_file_sorts_after_mains_newest_and_carries_a_down_section() {
    let (_, name, text) = the_file();
    let all = names();
    assert!(all.iter().any(|n| n == BEFORE), "{BEFORE} is on main");
    assert!(name.as_str() > BEFORE, "{name} sorts after {BEFORE}");
    assert!(
        all.iter()
            .all(|n| !n.contains(".up.") && !n.contains(".down.")),
        "no reversible migration: {all:?}"
    );
    down_sql(&text);
}

// ── the backfill ────────────────────────────────────────────────────────────

/// Ticket §9 row 1: every OPEN shift's `expected` is its drawer balance after the backfill; each safe is its
/// `ops_state.safe`; a CLOSED shift's drawer holds nothing.
#[tokio::test]
async fn every_open_drawer_and_every_safe_reads_as_v2_after_the_backfill() {
    let db = backfilled().await;
    assert_eq!(
        balances(&db, CashPlace::Drawer("SH-M1".into())).await,
        map(&[("THB", "10000.00"), ("USD", "500.00"), ("JPY", "20000.00")]),
        "SH-M1's drawer = its shift_balances.expected"
    );
    assert_eq!(
        balances(&db, CashPlace::Drawer("SH-M2".into())).await,
        map(&[("THB", "5000.00")]),
        "SH-M2 (B3): a zero expected is no balance"
    );
    assert_eq!(
        balances(&db, CashPlace::Drawer("SH-B1".into())).await,
        map(&[("USD", "100.00")])
    );
    assert_eq!(
        balances(&db, CashPlace::Drawer("SH-MC".into())).await,
        map(&[]),
        "a CLOSED shift is history: nothing opened on its drawer"
    );
    assert_eq!(
        balances(&db, CashPlace::Safe(BranchId::new("MAIN"))).await,
        map(&[("THB", "50000.00"), ("USD", "2000.00"), ("JPY", "10000.00")]),
        "MAIN's safe = ops_state.safe (minor units)"
    );
    assert_eq!(
        balances(&db, CashPlace::Safe(BranchId::new("B2"))).await,
        map(&[("USD", "300.00")])
    );
    db.finish().await;
}

#[tokio::test]
async fn every_backfilled_posting_is_opening() {
    let db = backfilled().await;
    assert!(
        count(&db, "ledger_postings").await > 0,
        "the backfill posts"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM ledger_postings WHERE source_type <> 'OPENING'")
            .await,
        0,
        "only OPENING postings"
    );
    db.finish().await;
}

/// Q-3: lots at the published BUY rate ÷ unit size; positions = the branch's safe plus its OPEN drawers.
#[tokio::test]
async fn foreign_balances_become_positions_costed_at_the_published_buy_rate() {
    let db = backfilled().await;
    let positions: Vec<(String, String, Decimal, Decimal)> = sqlx::query_as(
        "SELECT branch_id, trim(currency), quantity, cost_thb FROM inventory_positions ORDER BY 1, 2",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let round2 = |v: Decimal| v.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    let want = vec![
        (
            "B2".to_owned(),
            "USD".to_owned(),
            d("400"),
            round2(d("400") * d("32.40")),
        ),
        (
            "MAIN".to_owned(),
            "JPY".to_owned(),
            d("30000"),
            round2(d("30000") * d("0.235")),
        ),
        (
            "MAIN".to_owned(),
            "USD".to_owned(),
            d("2500"),
            round2(d("2500") * d("32.10")),
        ),
    ];
    let got: Vec<(String, String, Decimal, Decimal)> = positions
        .into_iter()
        .map(|(b, c, q, k)| (b, c, q.normalize(), k))
        .collect();
    assert_eq!(
        got,
        want.into_iter()
            .map(|(b, c, q, k)| (b, c, q.normalize(), k))
            .collect::<Vec<_>>(),
        "one position per branch and foreign currency: safe + OPEN drawers, at BUY ÷ unit_size; no THB, \
         the CLOSED shift's USD 999 not counted"
    );
    for (branch, currency, unit) in [
        ("MAIN", "USD", "32.10"),
        ("MAIN", "JPY", "0.235"),
        ("B2", "USD", "32.40"),
    ] {
        let lots: Vec<(Decimal, Decimal)> = sqlx::query_as(
            "SELECT quantity, unit_cost_thb FROM inventory_lots WHERE branch_id = $1 AND trim(currency) = $2",
        )
        .bind(branch)
        .bind(currency)
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert!(!lots.is_empty(), "{branch} {currency} has a lot");
        for (_, unit_cost) in &lots {
            assert_eq!(
                unit_cost.normalize(),
                d(unit)
                    .round_dp_with_strategy(8, RoundingStrategy::MidpointAwayFromZero)
                    .normalize(),
                "{branch} {currency}: BUY ÷ unit_size"
            );
        }
    }
    assert_eq!(
        db.count("SELECT count(*) FROM inventory_lots WHERE trim(currency) = 'THB'")
            .await
            + db.count("SELECT count(*) FROM inventory_positions WHERE trim(currency) = 'THB'")
                .await,
        0,
        "THB is not inventory"
    );
    db.finish().await;
}

/// Ticket §9 row 1: reconciliation (§6) checks 1, 2, 3 and 5 are clean after the backfill.
#[tokio::test]
async fn reconciliation_is_clean_after_the_backfill() {
    let db = backfilled().await;
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
    for id in [1u8, 2, 3, 5] {
        let check = report
            .checks
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("check {id} is registered: {report:?}"));
        assert!(check.ok, "check {id} is clean: {:?}", check.offenders);
    }
    db.finish().await;
}

#[tokio::test]
async fn the_v2_rows_are_read_and_not_changed() {
    let (db, _) = before_the_backfill(false).await;
    let before = (
        snapshot(&db, "shift_balances").await,
        snapshot(&db, "stock_movements").await,
        snapshot(&db, "ops_state").await,
    );
    run_the_rest(&db).await.expect("migration 2 applies");
    assert_eq!(
        (
            snapshot(&db, "shift_balances").await,
            snapshot(&db, "stock_movements").await,
            snapshot(&db, "ops_state").await,
        ),
        before,
        "shift_balances, stock_movements and ops_state are read, not changed"
    );
    db.finish().await;
}

/// Ticket §9 row 2: no published BUY rate for a currency held ⇒ the migration stops naming it; nothing is written.
#[tokio::test]
async fn a_currency_without_a_published_buy_rate_stops_the_migration_naming_it() {
    let (db, version) = before_the_backfill(true).await;
    let err = run_the_rest(&db)
        .await
        .expect_err("GBP is held at MAIN with no published BUY rate");
    assert!(err.contains("GBP"), "the error names the currency: {err}");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM _sqlx_migrations WHERE version = {version} AND success"
        ))
        .await,
        0,
        "migration 2 is not recorded as applied"
    );
    for table in [
        "ledger_postings",
        "ledger_entries",
        "inventory_lots",
        "inventory_positions",
        "inventory_movements",
    ] {
        assert_eq!(count(&db, table).await, 0, "nothing written to {table}");
    }
    db.finish().await;
}

// ── down ────────────────────────────────────────────────────────────────────

async fn run_down(db: &TestDb) -> Result<(), String> {
    let (_, _, text) = the_file();
    sqlx::raw_sql(AssertSqlSafe(down_sql(&text)))
        .execute(&db.pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Ticket §9 row 5 / §10 item 4: `down` is refused once any posting other than `OPENING` exists.
#[tokio::test]
async fn down_is_refused_once_a_non_opening_posting_exists() {
    let db = backfilled().await;
    let mut tx = db.pool.begin().await.unwrap();
    LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Safe(BranchId::new("MAIN")),
                to: CashPlace::Drawer("SH-M1".into()),
                currency: CurrencyCode::THB,
                quantity: ForeignAmount::from_decimal(d("100.00")),
                source: SourceRef {
                    source_type: SourceType::Transfer,
                    source_id: "after-cutover".into(),
                    source_key: "dispatch".into(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("a TRANSFER posting after the cutover");
    tx.commit().await.unwrap();
    let before: Vec<i64> = {
        let mut v = Vec::new();
        for t in [
            "ledger_postings",
            "ledger_entries",
            "inventory_lots",
            "inventory_positions",
            "inventory_movements",
        ] {
            v.push(count(&db, t).await);
        }
        v
    };
    let refused = run_down(&db).await;
    assert!(
        refused.is_err(),
        "down is refused after a non-OPENING posting"
    );
    let after: Vec<i64> = {
        let mut v = Vec::new();
        for t in [
            "ledger_postings",
            "ledger_entries",
            "inventory_lots",
            "inventory_positions",
            "inventory_movements",
        ] {
            v.push(count(&db, t).await);
        }
        v
    };
    assert_eq!(after, before, "a refused down deletes nothing");
    db.finish().await;
}

/// §10 item 4: with only `OPENING` postings, down deletes the movements and entries, the lots, the postings and
/// every position; the v2 tables are left as they were.
#[tokio::test]
async fn down_deletes_the_backfill_when_only_opening_postings_exist() {
    let db = backfilled().await;
    let v2 = (
        snapshot(&db, "shift_balances").await,
        snapshot(&db, "ops_state").await,
    );
    run_down(&db)
        .await
        .expect("down runs with only OPENING postings");
    for table in [
        "ledger_postings",
        "ledger_entries",
        "inventory_lots",
        "inventory_positions",
        "inventory_movements",
    ] {
        assert_eq!(count(&db, table).await, 0, "down empties {table}");
    }
    assert_eq!(
        (
            snapshot(&db, "shift_balances").await,
            snapshot(&db, "ops_state").await,
        ),
        v2,
        "the v2 tables are untouched"
    );
    db.finish().await;
}

#[test]
fn the_currency_helpers_parse() {
    // keeps `cur` used: the codes this file seeds are valid ISO codes
    for c in ["THB", "USD", "JPY", "GBP"] {
        cur(c);
    }
}
