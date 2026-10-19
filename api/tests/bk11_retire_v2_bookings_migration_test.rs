//! Ticket #1413 (BK-11): schema M2, retiring v2's booking (design `docs/design/blueprint/booking-slots.md` §6, §8 M2
//! items 1–4, §9.1 row BK-11; the owner's release on #1413, 2026-10-17). Written before the implementation by the
//! ticket's test writer (claude/5781a09c, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What this test pins ([required]):
//! - one `api/migrations/*_retire_v2_bookings.sql`, after `main`'s newest at writing (`20261205202000`) and after
//!   M1 (`*_booking_slots.sql`), with a `-- down:` section; every migration applies in version order on a fresh
//!   database, and the result has neither `v2_bookings` nor `members` (item 1);
//! - v2 rows (bookings in each v2 status, a member) are dropped with their tables (item 1);
//! - the `bookingRules` rows of `branch_settings` and `branch_setting_history` are deleted, and every other setting
//!   and its history stays byte for byte (item 1);
//! - a `transactions.booking_id` that names a v2 booking becomes `NULL`, and nothing else about that row changes
//!   (item 1);
//! - after M2, `transactions.booking_id` can name a Blueprint `bookings` id and cannot name an id that is not in
//!   `bookings` (the FK is re-pointed at `bookings(id)`, item 1);
//! - after M2 no foreign key in the schema references a table named `v2_bookings` or `members`, and the
//!   `queue_tickets` and `referral_attributions` keys still reference `bookings` (M1 item 1 is kept);
//! - `down` re-creates `v2_bookings` and `members` empty and points `transactions.booking_id` back at
//!   `v2_bookings` (item 4).
//!
//! Each case migrates through the version before M2, seeds, then runs the migrations **up to and including M2
//! only**, so a later migration cannot reach the shape these tests seed. Needs `TEST_DATABASE_URL`; nothing
//! here skips.

mod quote_common;

use std::path::{Path, PathBuf};

use quote_common::{ORG_1, TestDb, unique};
use sqlx::AssertSqlSafe;
use sqlx::migrate::Migrator;
use uuid::Uuid;

const SUFFIX: &str = "_retire_v2_bookings.sql";
const M1_SUFFIX: &str = "_booking_slots.sql";
/// `main`'s newest migration when these tests were written (`cd0e6dba`).
const MAINS_NEWEST_AT_WRITING: i64 = 20261205202000;
const MAIN: &str = "MAIN";

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

fn version_of(name: &str) -> i64 {
    name[..name.find('_').expect("<version>_")]
        .parse()
        .expect("a numeric version")
}

/// (version, file name, text) of the one migration ending in `suffix`.
fn one_file(suffix: &str) -> (i64, String, String) {
    let found: Vec<String> = names()
        .into_iter()
        .filter(|n| n.ends_with(suffix))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/*{suffix} (design §8): {found:?}"
    );
    let name = found[0].clone();
    let text = std::fs::read_to_string(migrations().join(&name)).expect("read the migration");
    (version_of(&name), name, text)
}

/// (version, file name, text) of the one M2.
fn the_file() -> (i64, String, String) {
    one_file(SUFFIX)
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

/// A database at the version before M2.
async fn before_m2() -> TestDb {
    let (version, _, _) = the_file();
    let db = TestDb::empty().await;
    db.migrate_through(Some(version_before(version))).await;
    db
}

/// The migrations after the seed, up to and including M2 and nothing later; the error text when one fails.
async fn run_m2(db: &TestDb) -> Result<(), String> {
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

async fn after_m2() -> TestDb {
    let db = before_m2().await;
    run_m2(&db).await.expect("M2 applies");
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

async fn exists(db: &TestDb, relation: &str) -> bool {
    db.text(&format!("SELECT to_regclass('{relation}')::text"))
        .await
        .is_some()
}

/// The table that the foreign key on `table(column)` references, if there is one.
async fn fk_target(db: &TestDb, table: &str, column: &str) -> Option<String> {
    db.text(&format!(
        "SELECT c.confrelid::regclass::text FROM pg_constraint c \
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey) \
         WHERE c.contype = 'f' AND c.conrelid = '{table}'::regclass AND a.attname = '{column}'"
    ))
    .await
}

/// A refused statement's error text (the statement must fail).
async fn refused(db: &TestDb, sql: &str) -> String {
    sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(&db.pool)
        .await
        .map(|_| ())
        .expect_err(sql)
        .to_string()
}

// ── seeds (the schema at the version before M2) ─────────────────────────────

/// A customer of MAIN's organisation.
async fn customer(db: &TestDb) -> String {
    let id = unique("CU");
    db.customer(&id, MAIN).await;
    id
}

/// A staff-verified v2 member of MAIN for `customer`.
async fn member(db: &TestDb, customer: &str) -> String {
    let id = unique("MB");
    sqlx::query(
        "INSERT INTO members (id, branch_id, customer_id, name, phone, token_hash, verified_at, verified_by) \
         VALUES ($1, $2, $3, 'Member', '0812345678', $4, now(), 'owner')",
    )
    .bind(&id)
    .bind(MAIN)
    .bind(customer)
    .bind(unique("token"))
    .execute(&db.pool)
    .await
    .expect("insert a member");
    id
}

/// One v2 booking of MAIN in each v2 status, as v2 writes them into `v2_bookings`. Answers their ids.
async fn v2_bookings(db: &TestDb) -> Vec<String> {
    let c = customer(db).await;
    let m = member(db, &c).await;
    let mut ids = Vec::new();
    for (status, member_id, expires) in [
        ("PENDING", Some(m.as_str()), false),
        ("CONFIRMED", None, true),
        ("CANCELLED", None, false),
    ] {
        let id = format!("BK-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO v2_bookings (id, branch_id, request_id, request_hash, name, phone, member_id, type, code, \
                 amount, rate, status, created_at, created_by, expires_at) \
             VALUES ($1, $2, $3, 'h', 'Name', '0899999999', $4, 'SELL', 'USD', 100, 32.6, $5, now(), 'owner', \
                 CASE WHEN $6 THEN now() + interval '1 hour' END)",
        )
        .bind(&id)
        .bind(MAIN)
        .bind(unique("req"))
        .bind(member_id)
        .bind(status)
        .bind(expires)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert a v2 {status} booking: {e}"));
        ids.push(id);
    }
    ids
}

/// A stored DRAFT DEMO exchange of MAIN for `customer`, naming `booking_id`. Answers its id.
async fn exchange(db: &TestDb, customer: &str, booking_id: Option<&str>) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO transactions (branch_id, mode, submission_id, payload_hash, \
             customer_id, customer_name, nationality, staff_id, booking_no, notes, \
             value_date, paid_at, recorded_at, recorded_by, status, booking_id) \
         VALUES ($1, 'DEMO', $2, 'payload-hash', $3, 'Customer', 'THA', 'owner', '', '', \
             DATE '2026-10-12', now(), now(), 'owner', 'DRAFT', $4) RETURNING id",
    )
    .bind(MAIN)
    .bind(unique("submission"))
    .bind(customer)
    .bind(booking_id)
    .fetch_one(&db.pool)
    .await
    .expect("insert an exchange")
}

/// A `branch_settings` row of MAIN named `name` with one history row. Answers nothing.
async fn setting(db: &TestDb, name: &str, body: &str) {
    db.exec(&format!(
        "INSERT INTO branch_settings (branch_id, name, version, body, updated_at, updated_by) \
         VALUES ('{MAIN}', '{name}', 1, '{body}'::jsonb, now(), 'owner')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO branch_setting_history (branch_id, name, version, actor, at, before, after) \
         VALUES ('{MAIN}', '{name}', 1, 'owner', now(), '{{}}'::jsonb, '{body}'::jsonb)"
    ))
    .await;
}

/// An OPEN slot of MAIN. Answers its id.
async fn slot(db: &TestDb) -> Uuid {
    let id = Uuid::new_v4();
    let start = chrono::Utc::now()
        + chrono::Duration::days(1)
        + chrono::Duration::seconds(i64::from((Uuid::new_v4().as_u128() % 1_000_000) as u32));
    sqlx::query(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, \
             reserved, status) \
         VALUES ($1, $2::uuid, $3, ($4::timestamptz)::date, $4, $4 + interval '30 minutes', 4, 0, 'OPEN')",
    )
    .bind(id)
    .bind(ORG_1)
    .bind(MAIN)
    .bind(start)
    .execute(&db.pool)
    .await
    .expect("insert a slot");
    id
}

/// A CONFIRMED DESK Blueprint booking of MAIN for `customer`. Answers its id.
async fn new_booking(db: &TestDb, customer: &str) -> String {
    let s = slot(db).await;
    let id = format!("BK-{}", Uuid::new_v4());
    db.exec(&format!(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
             requested_currency, requested_quantity, source_channel, status, qr_token_hash, created_by) \
         VALUES ('{id}', '{ORG_1}', '{MAIN}', '{no}', '{customer}', '{s}', 'BUY', 'USD', 100, 'DESK', \
             'CONFIRMED', decode('{qr:032x}', 'hex'), 'test')",
        no = unique("B"),
        qr = Uuid::new_v4().as_u128(),
    ))
    .await;
    id
}

// ── the file and the order ──────────────────────────────────────────────────

#[test]
fn m2_sorts_after_mains_newest_and_after_m1_and_carries_a_down_section() {
    let (version, name, text) = the_file();
    let (m1, m1_name, _) = one_file(M1_SUFFIX);
    assert!(
        version > MAINS_NEWEST_AT_WRITING,
        "{name} sorts after main's newest at writing ({MAINS_NEWEST_AT_WRITING})"
    );
    assert!(
        version > m1,
        "{name} sorts after M1 {m1_name} (design §8: M1 comes before M2)"
    );
    down_sql(&text);
}

#[tokio::test]
async fn every_migration_applies_in_order_and_leaves_no_v2_booking_table() {
    let db = TestDb::start().await;
    for table in ["v2_bookings", "members"] {
        assert!(!exists(&db, table).await, "{table} is dropped by M2");
    }
    for table in ["bookings", "booking_slots", "booking_events"] {
        assert!(exists(&db, table).await, "the Blueprint {table} stays");
    }
    db.finish().await;
}

// ── item 1: existing rows ───────────────────────────────────────────────────

#[tokio::test]
async fn v2_rows_go_with_their_tables() {
    let db = before_m2().await;
    let ids = v2_bookings(&db).await;
    assert_eq!(ids.len(), 3);
    assert_eq!(db.count("SELECT count(*) FROM members").await, 1);

    run_m2(&db).await.expect("M2 applies over v2 rows");

    assert!(!exists(&db, "v2_bookings").await, "v2_bookings is dropped");
    assert!(!exists(&db, "members").await, "members is dropped");
    db.finish().await;
}

#[tokio::test]
async fn booking_rules_settings_and_their_history_go_and_other_settings_stay() {
    let db = before_m2().await;
    setting(&db, "bookingRules", r#"{"enabled": true, "minutes": 30}"#).await;
    setting(&db, "receiptFooter", r#"{"text": "thank you"}"#).await;
    let other_before = db
        .text(
            "SELECT body::text || '|' || version::text FROM branch_settings \
             WHERE branch_id = 'MAIN' AND name = 'receiptFooter'",
        )
        .await;
    let other_history_before = db
        .text(
            "SELECT after::text FROM branch_setting_history \
             WHERE branch_id = 'MAIN' AND name = 'receiptFooter'",
        )
        .await;

    run_m2(&db).await.expect("M2 applies");

    assert_eq!(
        db.count("SELECT count(*) FROM branch_settings WHERE name = 'bookingRules'")
            .await,
        0,
        "the bookingRules settings are deleted"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM branch_setting_history WHERE name = 'bookingRules'")
            .await,
        0,
        "the bookingRules history is deleted"
    );
    assert_eq!(
        db.text(
            "SELECT body::text || '|' || version::text FROM branch_settings \
             WHERE branch_id = 'MAIN' AND name = 'receiptFooter'",
        )
        .await,
        other_before,
        "another setting is unchanged"
    );
    assert_eq!(
        db.text(
            "SELECT after::text FROM branch_setting_history \
             WHERE branch_id = 'MAIN' AND name = 'receiptFooter'",
        )
        .await,
        other_history_before,
        "another setting's history is unchanged"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_transaction_naming_a_v2_booking_keeps_its_row_with_booking_id_null() {
    let db = before_m2().await;
    let ids = v2_bookings(&db).await;
    let c = customer(&db).await;
    let booked = exchange(&db, &c, Some(&ids[1])).await;
    let plain = exchange(&db, &c, None).await;
    let row = |id: Uuid| {
        format!(
            "SELECT concat_ws('|', branch_id, mode, submission_id, customer_id, status) \
                 FROM transactions WHERE id = '{id}'"
        )
    };
    let booked_before = db.text(&row(booked)).await;
    let plain_before = db.text(&row(plain)).await;

    run_m2(&db)
        .await
        .expect("M2 applies over a booked transaction");

    assert_eq!(
        db.text(&format!(
            "SELECT booking_id FROM transactions WHERE id = '{booked}'"
        ))
        .await,
        None,
        "a booking_id naming a v2 booking becomes NULL"
    );
    assert_eq!(
        db.text(&row(booked)).await,
        booked_before,
        "nothing else on that row changes"
    );
    assert_eq!(
        db.text(&row(plain)).await,
        plain_before,
        "a row with no booking is unchanged"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM transactions").await,
        2,
        "no transaction is deleted"
    );
    db.finish().await;
}

// ── item 1: the re-pointed key ──────────────────────────────────────────────

#[tokio::test]
async fn after_m2_transactions_booking_id_references_bookings() {
    let db = after_m2().await;
    assert_eq!(
        fk_target(&db, "transactions", "booking_id")
            .await
            .as_deref(),
        Some("bookings"),
        "transactions.booking_id references bookings(id)"
    );
    let c = customer(&db).await;
    let b = new_booking(&db, &c).await;
    exchange(&db, &c, Some(&b)).await;
    let err = refused(
        &db,
        &format!(
            "UPDATE transactions SET booking_id = 'BK-{}' WHERE customer_id = '{c}'",
            Uuid::new_v4()
        ),
    )
    .await;
    assert!(
        err.contains("23503") || err.contains("foreign key"),
        "an id that is not a Blueprint booking is refused by the FK: {err}"
    );
    db.finish().await;
}

#[tokio::test]
async fn after_m2_no_key_names_a_v2_table_and_m1s_keys_stay_on_bookings() {
    let db = after_m2().await;
    assert_eq!(
        db.count(
            "SELECT count(*) FROM pg_constraint c \
             JOIN pg_class t ON t.oid = c.confrelid \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             WHERE c.contype = 'f' AND n.nspname = current_schema() \
               AND t.relname IN ('v2_bookings', 'members')"
        )
        .await,
        0,
        "no foreign key references v2_bookings or members"
    );
    assert_eq!(
        fk_target(&db, "queue_tickets", "booking_id")
            .await
            .as_deref(),
        Some("bookings"),
        "queue_tickets.booking_id stays on bookings (M1 item 1)"
    );
    assert_eq!(
        fk_target(&db, "referral_attributions", "booking_id")
            .await
            .as_deref(),
        Some("bookings"),
        "referral_attributions.booking_id stays on bookings (M1 item 1)"
    );
    db.finish().await;
}

// ── item 4: down ────────────────────────────────────────────────────────────

#[tokio::test]
async fn down_recreates_the_v2_tables_empty_and_points_the_key_back() {
    let db = before_m2().await;
    v2_bookings(&db).await;
    run_m2(&db).await.expect("M2 applies");

    run_down(&db).await.expect("down applies");

    for table in ["v2_bookings", "members"] {
        assert!(exists(&db, table).await, "down re-creates {table}");
        assert_eq!(
            db.count(&format!("SELECT count(*) FROM {table}")).await,
            0,
            "{table} comes back empty: down does not restore rows"
        );
    }
    assert_eq!(
        fk_target(&db, "transactions", "booking_id")
            .await
            .as_deref(),
        Some("v2_bookings"),
        "down points transactions.booking_id back at v2_bookings"
    );
    assert!(
        exists(&db, "bookings").await,
        "the Blueprint bookings stays"
    );
    db.finish().await;
}
