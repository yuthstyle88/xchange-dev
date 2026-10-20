//! TX-9 #677: migration 3 drops the wide figure and payment columns of `transactions`.
//! Written before the implementation by the ticket author (claude/c4d72c), on the BE lane controller claude/7ca80c's
//! order of 2026-10-15. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/quote-transaction.md` §8 item 3, with the controller's rulings 1–3 on #677
//! (issuecomment-6183687076; recorded in the design by PR 1978):
//! - the file `api/migrations/<version>_drop_wide_transaction_columns.sql` sorts above main's newest at writing
//!   (`20261205202000`) and carries a `-- down:` section;
//! - it drops 15 columns: `type, code, amount, rate, reference_rate, unit_size, principal, fee, bank_fee, received, paid,
//!   method, payment_ref, account, published_rate_line_id`. **`booking_no` stays** (ruling 1);
//! - its precheck `RAISE`s (SQLSTATE P0001) while a `transactions` row has no `transaction_lines` row, and then changes
//!   nothing;
//! - it deletes a `transaction_files` SLIP row only where `file_key` equals that exchange's
//!   `transaction_payments.slip_file_key`; any other SLIP row, and every PASSPORT and RECEIPT row, is kept (ruling 2);
//! - `down` re-adds the 15 columns NULL-able and fills them from line 1 and payment 1 (ruling 3): the settling side
//!   (`received` for SELL, `paid` for BUY) = the line's `settle_thb`, the other side `0.00`; `published_rate_line_id`
//!   from `quote_line_id → quote_lines`, else NULL; and `up` applies again after `down`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

mod quote_common;

use std::path::{Path, PathBuf};

use quote_common::{ORG_1, TestDb, unique};
use sqlx::AssertSqlSafe;
use sqlx::migrate::Migrator;

const SUFFIX: &str = "_drop_wide_transaction_columns.sql";
/// `main`'s newest migration when these tests were written.
const MAINS_NEWEST_AT_WRITING: i64 = 20261205202000;
const MAIN: &str = "MAIN";
const DROPPED: [&str; 15] = [
    "type",
    "code",
    "amount",
    "rate",
    "reference_rate",
    "unit_size",
    "principal",
    "fee",
    "bank_fee",
    "received",
    "paid",
    "method",
    "payment_ref",
    "account",
    "published_rate_line_id",
];

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

/// (version, file name, text) of the one migration 3.
fn the_file() -> (i64, String, String) {
    let found: Vec<String> = names()
        .into_iter()
        .filter(|n| n.ends_with(SUFFIX))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/*{SUFFIX} (design §8 item 3): {found:?}"
    );
    let name = found[0].clone();
    let version: i64 = name[..name.find('_').expect("<version>_")]
        .parse()
        .expect("a numeric version");
    let text = std::fs::read_to_string(migrations().join(&name)).expect("read the migration");
    (version, name, text)
}

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

/// A database at the version before migration 3.
async fn before_m3() -> TestDb {
    let (version, _, _) = the_file();
    let db = TestDb::empty().await;
    db.migrate_through(Some(version_before(version))).await;
    db
}

/// The migrations after the seed, up to and including migration 3 and nothing later; the error text when one fails.
async fn run_m3(db: &TestDb) -> Result<(), String> {
    let (version, _, _) = the_file();
    let mut migrator = Migrator::new(migrations().as_path())
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

async fn after_m3() -> TestDb {
    let db = before_m3().await;
    run_m3(&db).await.expect("migration 3 applies");
    db
}

async fn run_down(db: &TestDb) -> Result<(), String> {
    let (_, _, text) = the_file();
    sqlx::raw_sql(AssertSqlSafe(down_sql(&text)))
        .execute(&db.pool)
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:?} / {e}"))
}

async fn has_column(db: &TestDb, table: &str, column: &str) -> bool {
    db.count(&format!(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = '{table}' AND column_name = '{column}'"
    ))
    .await
        == 1
}

async fn file(db: &TestDb, key: &str) {
    db.exec(&format!(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ('{key}', '{MAIN}', 'test', 'image/jpeg', 'x', now())"
    ))
    .await;
}

/// A COMPLETED DEMO exchange (no shift needed) at MAIN on the schema before migration 3: the wide row (as the writers wrote it), its one line
/// and its one payment. `side` BUY or SELL; `slip` the payment's slip key (also a SLIP `transaction_files` row named
/// `slip_row`, when given). Returns the exchange's id.
async fn exchange(
    db: &TestDb,
    side: &str,
    slip: Option<&str>,
    slip_row: Option<&str>,
    with_line: bool,
) -> String {
    let customer = unique("cx");
    db.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ('{customer}', '{ORG_1}', '{customer}', 'Somchai', 'LIVE', now())"
    ))
    .await;
    let id = db
        .text(&format!(
            "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, submission_id, payload_hash, \
               customer_id, customer_name, nationality, type, code, amount, rate, reference_rate, unit_size, \
               principal, fee, bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
               value_date, paid_at, recorded_at, completed_at, recorded_by) \
             VALUES (gen_random_uuid(), '{ORG_1}', '{doc}', '{MAIN}', 'DEMO', '{sub}', 'hash', '{customer}', \
               'Somchai', 'TH', '{side}', 'USD', 100, 35, 34.5, 1, 3500, 10, 5, \
               {received}, {paid}, 'CASH', 'ref-1', 'acc-1', 'staff', 'WALK-IN', '', CURRENT_DATE, now(), now(), now(), \
               'staff') RETURNING id::text",
            doc = unique("XC"),
            sub = unique("sub"),
            received = if side == "SELL" { "3490" } else { "0" },
            paid = if side == "BUY" { "3490" } else { "0" },
        ))
        .await
        .expect("an exchange id");
    if with_line {
        db.exec(&format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, \
               unit_size, rate_snapshot, reference_rate, thb_amount, fee_thb, settle_thb, line_no, created_at, \
               updated_at) \
             VALUES (gen_random_uuid(), '{ORG_1}', '{id}', '{side}', 'USD', 100, 1, 35, 34.5, 3500, 10, 3490, 1, \
               now(), now())"
        ))
        .await;
    }
    if let Some(key) = slip {
        file(db, key).await;
    }
    db.exec(&format!(
        "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, reference, \
           account, slip_file_key, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{id}', 1, 'CASH', 'ref-1', 'acc-1', {slip_sql}, 3490, 5, now(), now(), \
           now())",
        slip_sql = slip.map(|k| format!("'{k}'")).unwrap_or_else(|| "NULL".to_owned()),
    ))
    .await;
    if let Some(key) = slip_row {
        if Some(key) != slip {
            file(db, key).await;
        }
        db.exec(&format!(
            "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ('{id}', 'SLIP', '{key}')"
        ))
        .await;
    }
    id
}

#[test]
fn migration_3_sorts_above_mains_newest_and_carries_a_down_section() {
    let (version, name, text) = the_file();
    assert!(
        version > MAINS_NEWEST_AT_WRITING,
        "{name} sorts above {MAINS_NEWEST_AT_WRITING}"
    );
    down_sql(&text);
}

#[tokio::test]
async fn it_drops_the_fifteen_columns_and_keeps_booking_no() {
    let db = after_m3().await;
    for column in DROPPED {
        assert!(
            !has_column(&db, "transactions", column).await,
            "{column} is dropped"
        );
    }
    assert!(
        has_column(&db, "transactions", "booking_no").await,
        "booking_no stays (ruling 1)"
    );
    assert!(
        has_column(&db, "transactions", "status").await,
        "the kept columns stay"
    );
    db.finish().await;
}

#[tokio::test]
async fn it_refuses_while_an_exchange_has_no_line_and_changes_nothing() {
    let db = before_m3().await;
    exchange(&db, "BUY", None, None, false).await;

    let err = run_m3(&db).await.expect_err("migration 3 refuses");
    assert!(
        err.contains("P0001"),
        "its own RAISE (SQLSTATE P0001): {err}"
    );
    for column in DROPPED {
        assert!(
            has_column(&db, "transactions", column).await,
            "{column} is still there"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn it_deletes_only_the_slip_rows_that_match_their_payment() {
    let db = before_m3().await;
    let matched = unique("slip-match");
    let unmatched_row = unique("slip-row");
    let unmatched_payment = unique("slip-pay");
    let a = exchange(&db, "BUY", Some(&matched), Some(&matched), true).await;
    let b = exchange(
        &db,
        "SELL",
        Some(&unmatched_payment),
        Some(&unmatched_row),
        true,
    )
    .await;
    let passport = unique("passport");
    file(&db, &passport).await;
    db.exec(&format!(
        "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ('{a}', 'PASSPORT', '{passport}')"
    ))
    .await;

    run_m3(&db).await.expect("migration 3 applies");
    let slip_of = |id: &str| {
        format!(
            "SELECT count(*) FROM transaction_files WHERE transaction_id = '{id}' AND kind = 'SLIP'"
        )
    };
    assert_eq!(
        db.count(&slip_of(&a)).await,
        0,
        "a SLIP row matching its payment's slip_file_key goes"
    );
    assert_eq!(
        db.count(&slip_of(&b)).await,
        1,
        "a SLIP row with no match is kept"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM transaction_files WHERE transaction_id = '{a}' AND kind = 'PASSPORT'"
        ))
        .await,
        1,
        "PASSPORT rows are kept"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM transaction_payments WHERE slip_file_key = '{matched}'"
        ))
        .await,
        1,
        "the payment keeps its slip key"
    );
    db.finish().await;
}

#[tokio::test]
async fn down_refills_the_columns_from_the_children_and_up_applies_again() {
    let db = before_m3().await;
    let buy = exchange(&db, "BUY", None, None, true).await;
    let sell = exchange(&db, "SELL", None, None, true).await;
    run_m3(&db).await.expect("migration 3 applies");
    run_down(&db).await.expect("down applies");

    for column in DROPPED {
        assert!(
            has_column(&db, "transactions", column).await,
            "down re-adds {column}"
        );
        assert_eq!(
            db.text(&format!(
                "SELECT is_nullable FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = 'transactions' AND column_name = '{column}'"
            ))
            .await
            .as_deref(),
            Some("YES"),
            "{column} comes back NULL-able"
        );
    }
    let row = |id: &str| {
        format!(
            "SELECT concat_ws('|', type, trim(code), amount::numeric(20,2), rate::numeric(14,4), \
               reference_rate::numeric(14,4), unit_size::numeric(20,4), principal, fee, bank_fee, received, paid, \
               method, payment_ref, account, coalesce(published_rate_line_id::text, 'NULL')) \
             FROM transactions WHERE id = '{id}'"
        )
    };
    assert_eq!(
        db.text(&row(&buy)).await.as_deref(),
        Some(
            "BUY|USD|100.00|35.0000|34.5000|1.0000|3500.00|10.00|5.00|0.00|3490.00|CASH|ref-1|acc-1|NULL"
        ),
        "BUY: paid = the line's settle_thb, received 0.00; the rest from line 1 and payment 1"
    );
    assert_eq!(
        db.text(&row(&sell)).await.as_deref(),
        Some(
            "SELL|USD|100.00|35.0000|34.5000|1.0000|3500.00|10.00|5.00|3490.00|0.00|CASH|ref-1|acc-1|NULL"
        ),
        "SELL: received = the line's settle_thb, paid 0.00"
    );

    // up again after down
    db.exec(&format!(
        "DELETE FROM _sqlx_migrations WHERE version = {}",
        the_file().0
    ))
    .await;
    run_m3(&db)
        .await
        .expect("migration 3 applies again after down");
    for column in DROPPED {
        assert!(
            !has_column(&db, "transactions", column).await,
            "{column} dropped again"
        );
    }
    db.finish().await;
}
