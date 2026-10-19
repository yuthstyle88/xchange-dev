//! Ticket #1377 (BK-1): schema M1 of the Blueprint bookings (design `docs/design/blueprint/booking-slots.md` §4.1
//! tables, §8 M1 items 1–4, §9.1 row BK-1; the lane controller's rulings on #1377, issuecomment-6116674511, and
//! the design amendment of PR 1448 for ruling 2). Written before the implementation by the ticket author
//! (claude/89bd61d9, rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! What this test pins ([required]):
//! - one `api/migrations/*_booking_slots.sql`, after `main`'s newest at writing (`20261112110000`), with a
//!   `-- down:` section; every migration applies in version order on a fresh database;
//! - one `booking_policies` row per organisation existing at M1 (one with no branch too): 60 / 15 / 60,
//!   `updated_by` NULL (§0 Q-B2..Q-B4);
//! - v2's rows, one in each v2 status, survive the rename as `v2_bookings`, unchanged, and `bookings` is the new
//!   table;
//! - after M1 a `queue_tickets` row and a `referral_attributions` row can name a new `bookings` id and cannot
//!   name a `v2_bookings` id (both FKs re-pointed, §8 M1 item 1);
//! - M1 refuses with its own RAISE (SQLSTATE P0001) while a `queue_tickets.booking_id` or a
//!   `referral_attributions.booking_id` is set, and changes nothing;
//! - `down` refuses with its own RAISE while either names a new booking, and otherwise restores v2's `bookings` (item 4);
//! - the new schema refuses `reserved > capacity`, a `COMPLETED` booking without `transaction_id`, a second
//!   booking with the same `transaction_id`, a booking whose slot is another branch's, and a `PARTNER` booking
//!   without `partner_attribution_id` (ruling 3); `booking_events` is append only (§4.1, as consents).
//!
//! The booking capture's lookup on the new `bookings` (PR 1448) is pinned in
//! `crates/xc-referral/tests/capture_test.rs`.
//!
//! Each case migrates through the version before M1, seeds, then runs the migrations **up to and including M1
//! only**, so a later migration cannot reach the v2 shape these tests seed. Needs `TEST_DATABASE_URL`; nothing
//! here skips.

mod quote_common;

use std::path::{Path, PathBuf};

use quote_common::{ORG_1, TestDb, unique};
use sqlx::AssertSqlSafe;
use sqlx::migrate::Migrator;
use uuid::Uuid;

const SUFFIX: &str = "_booking_slots.sql";
const M2_SUFFIX: &str = "_retire_v2_bookings.sql";
/// `main`'s newest migration when these tests were written.
const MAINS_NEWEST_AT_WRITING: i64 = 20261112110000;
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

/// (version, file name, text) of the one M1.
fn the_file() -> (i64, String, String) {
    let found: Vec<String> = names()
        .into_iter()
        .filter(|n| n.ends_with(SUFFIX))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/*{SUFFIX} (design §8 M1): {found:?}"
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

fn m2_version() -> i64 {
    let mut found = names().into_iter().filter(|n| n.ends_with(M2_SUFFIX));
    let name = found.next().expect("the M2 migration");
    name[..name.find('_').expect("<version>_")]
        .parse()
        .expect("a numeric version")
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

/// A database at the version before M1.
async fn before_m1() -> TestDb {
    let (version, _, _) = the_file();
    let db = TestDb::empty().await;
    db.migrate_through(Some(version_before(version))).await;
    db
}

/// The migrations after the seed, up to and including M1 and nothing later; the error text when one fails.
async fn run_m1(db: &TestDb) -> Result<(), String> {
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

async fn after_m1() -> TestDb {
    let db = before_m1().await;
    run_m1(&db).await.expect("M1 applies");
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

async fn has_column(db: &TestDb, table: &str, column: &str) -> bool {
    db.count(&format!(
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = '{table}' AND column_name = '{column}'"
    ))
    .await
        == 1
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

// ── seeds ───────────────────────────────────────────────────────────────────

/// A customer of MAIN's organisation.
async fn customer(db: &TestDb) -> String {
    let id = unique("CU");
    db.customer(&id, MAIN).await;
    id
}

/// A staff-verified member of MAIN for `customer`.
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

/// A stored DEMO exchange of MAIN; `completed` gives it a document_ref. Answers (id, document_ref).
async fn exchange(db: &TestDb, customer: &str, completed: bool) -> (Uuid, Option<String>) {
    let document_ref = completed.then(|| unique("DOC"));
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, \
             customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
             bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
             value_date, paid_at, recorded_at, recorded_by, status, completed_at) \
         VALUES ($1, $2, 'DEMO', $3, 'payload-hash', $4, 'Customer', 'THA', 'SELL', 'USD', \
             100, 32.6, 3260, 0, 0, 3260, 0, 'Cash', '', '', 'owner', '', '', \
             DATE '2026-10-12', now(), now(), 'owner', $5, $6) RETURNING id",
    )
    .bind(&document_ref)
    .bind(MAIN)
    .bind(unique("submission"))
    .bind(customer)
    .bind(if completed { "COMPLETED" } else { "DRAFT" })
    .bind(completed.then(chrono::Utc::now))
    .fetch_one(&db.pool)
    .await
    .expect("insert an exchange");
    (id, document_ref)
}

/// One v2 booking of MAIN in each v2 status, as v2 writes them. Answers their ids.
async fn v2_bookings(db: &TestDb) -> Vec<String> {
    let c = customer(db).await;
    let m = member(db, &c).await;
    let (_, used_doc) = exchange(db, &c, true).await;
    let mut ids = Vec::new();
    for (status, member_id, expires, submission, transaction_ref) in [
        ("PENDING", Some(m.as_str()), false, None, None),
        ("CONFIRMED", None, true, None, None),
        (
            "USED",
            None,
            true,
            Some("submission-used"),
            used_doc.as_deref(),
        ),
        ("CANCELLED", None, false, None, None),
    ] {
        let id = format!("BK-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, member_id, type, code, \
                 amount, rate, status, created_at, created_by, expires_at, submission_id, transaction_ref) \
             VALUES ($1, $2, $3, 'h', 'Name', '0899999999', $4, 'SELL', 'USD', 100, 32.6, $5, now(), 'owner', \
                 CASE WHEN $6 THEN now() + interval '1 hour' END, $7, $8)",
        )
        .bind(&id)
        .bind(MAIN)
        .bind(unique("req"))
        .bind(member_id)
        .bind(status)
        .bind(expires)
        .bind(submission)
        .bind(transaction_ref)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert a v2 {status} booking: {e}"));
        ids.push(id);
    }
    ids
}

/// An OPEN slot of `branch` (after M1). Answers its id.
async fn slot(db: &TestDb, org: &str, branch: &str, capacity: i32) -> Uuid {
    let id = Uuid::new_v4();
    let start = chrono::Utc::now()
        + chrono::Duration::days(1)
        + chrono::Duration::seconds(i64::from(rand_offset()));
    sqlx::query(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, \
             reserved, status) \
         VALUES ($1, $2::uuid, $3, ($4::timestamptz)::date, $4, $4 + interval '30 minutes', $5, 0, 'OPEN')",
    )
    .bind(id)
    .bind(org)
    .bind(branch)
    .bind(start)
    .bind(capacity)
    .execute(&db.pool)
    .await
    .expect("insert a slot");
    id
}

fn rand_offset() -> u32 {
    (Uuid::new_v4().as_u128() % 1_000_000) as u32
}

/// The SQL of a new booking (after M1), so a test can expect it to fail. `status`, `channel`,
/// `transaction` and `partner` vary the CHECKs under test.
#[allow(clippy::too_many_arguments)]
fn booking_sql(
    id: &str,
    org: &str,
    branch: &str,
    customer: &str,
    slot: Uuid,
    status: &str,
    channel: &str,
    transaction: Option<Uuid>,
    partner: Option<Uuid>,
) -> String {
    format!(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
             requested_currency, requested_quantity, source_channel, status, qr_token_hash, transaction_id, \
             partner_attribution_id, created_by) \
         VALUES ('{id}', '{org}', '{branch}', '{no}', '{customer}', '{slot}', 'BUY', 'USD', 100, '{channel}', \
             '{status}', decode('{qr:032x}', 'hex'), {transaction}, {partner}, 'test')",
        no = unique("B"),
        qr = Uuid::new_v4().as_u128(),
        transaction = transaction.map_or("NULL".to_owned(), |t| format!("'{t}'")),
        partner = partner.map_or("NULL".to_owned(), |p| format!("'{p}'")),
    )
}

/// A CONFIRMED DESK booking of MAIN (after M1). Answers its id.
async fn new_booking(db: &TestDb) -> String {
    let c = customer(db).await;
    let s = slot(db, ORG_1, MAIN, 4).await;
    let id = format!("BK-{}", Uuid::new_v4());
    db.exec(&booking_sql(
        &id,
        ORG_1,
        MAIN,
        &c,
        s,
        "CONFIRMED",
        "DESK",
        None,
        None,
    ))
    .await;
    id
}

/// A queue ticket of MAIN naming `booking`, with `seq` as its sequence (distinct per ticket in one test, so the
/// `UNIQUE (branch_id, service_date, service_type, sequence)` key never fires before the FK).
fn ticket_sql(booking: &str, seq: u32) -> String {
    format!(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
             display_number, status, issued_at, idempotency_key, booking_id) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{MAIN}', current_date, 'BOOKING', {seq}, 'B{seq:03}', 'WAITING', \
             now(), '{key}', '{booking}')",
        key = unique("ticket"),
    )
}

/// A referral capture of MAIN naming `booking`.
fn capture_sql(customer: &str, booking: &str) -> String {
    format!(
        "INSERT INTO referral_attributions (id, organization_id, branch_id, code, customer_id, booking_id, \
             source, captured_at, captured_by, status) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{MAIN}', 'AB12CD', '{customer}', '{booking}', 'BOOKING', now(), \
             'test', 'ACTIVE')"
    )
}

// ── the file and the order ──────────────────────────────────────────────────

#[test]
fn m1_sorts_after_mains_newest_and_carries_a_down_section() {
    let (version, name, text) = the_file();
    assert!(
        version > MAINS_NEWEST_AT_WRITING,
        "{name} sorts after main's newest at writing ({MAINS_NEWEST_AT_WRITING})"
    );
    down_sql(&text);
}

#[tokio::test]
async fn every_migration_applies_in_order_on_a_fresh_database() {
    let db = TestDb::empty().await;
    db.migrate_through(Some(version_before(m2_version()))).await;
    for table in [
        "v2_bookings",
        "booking_policies",
        "booking_slot_rules",
        "branch_closures",
        "booking_slots",
        "booking_sequences",
        "bookings",
        "booking_events",
        "bookings_one_per_transaction",
        "bookings_slot_holding",
        "bookings_branch_day",
    ] {
        assert!(exists(&db, table).await, "{table} exists");
    }
    assert!(
        has_column(&db, "bookings", "slot_id").await,
        "bookings is the Blueprint table"
    );
    assert!(
        has_column(&db, "v2_bookings", "member_id").await,
        "v2_bookings is v2's table"
    );
    db.finish().await;
}

// ── item 2: the policy backfill ─────────────────────────────────────────────

#[tokio::test]
async fn each_organisation_gets_one_policy_row_of_60_15_60() {
    let db = before_m1().await;
    // a second organisation with no branch at all
    let lonely = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
         VALUES ('{lonely}', '{code}', NULL, 'Asia/Bangkok', now())",
        code = unique("LONE")
    ))
    .await;
    run_m1(&db).await.expect("M1 applies");

    let organisations = db.count("SELECT count(*) FROM organizations").await;
    assert_eq!(
        db.count("SELECT count(*) FROM booking_policies").await,
        organisations,
        "one row per organisation"
    );
    assert_eq!(
        db.count(
            "SELECT count(*) FROM booking_policies WHERE cancel_cutoff_minutes = 60 AND \
             no_show_grace_minutes = 15 AND confirm_expiry_minutes = 60 AND updated_by IS NULL"
        )
        .await,
        organisations,
        "60 / 15 / 60, seeded by the migration"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM booking_policies WHERE organization_id = '{lonely}'"
        ))
        .await,
        1,
        "an organisation with no branch gets its row too"
    );
    db.finish().await;
}

// ── item 1: the rename ──────────────────────────────────────────────────────

#[tokio::test]
async fn v2_rows_in_every_status_survive_the_rename_unchanged() {
    let db = before_m1().await;
    let ids = v2_bookings(&db).await;
    let before = db
        .text("SELECT string_agg(row_to_json(b)::text, '|' ORDER BY b.id) FROM bookings b")
        .await
        .expect("v2 rows");
    run_m1(&db).await.expect("M1 applies");

    assert_eq!(
        db.text("SELECT string_agg(row_to_json(b)::text, '|' ORDER BY b.id) FROM v2_bookings b")
            .await
            .expect("the v2 rows under their new name"),
        before,
        "every v2 row, in every status, is unchanged"
    );
    assert_eq!(
        db.count("SELECT count(DISTINCT status) FROM v2_bookings")
            .await,
        4
    );
    assert_eq!(ids.len(), 4);
    assert_eq!(
        db.count("SELECT count(*) FROM bookings").await,
        0,
        "the new bookings starts empty"
    );
    db.finish().await;
}

// ── item 1: the re-pointed FKs ──────────────────────────────────────────────

#[tokio::test]
async fn a_queue_ticket_names_a_new_booking_and_not_a_v2_one() {
    let db = before_m1().await;
    let v2 = v2_bookings(&db).await;
    run_m1(&db).await.expect("M1 applies");
    let new = new_booking(&db).await;

    db.exec(&ticket_sql(&new, 1)).await;
    let err = refused(&db, &ticket_sql(&v2[1], 2)).await;
    assert!(
        err.contains("queue_tickets_booking_id_fkey"),
        "a v2 id breaks the re-pointed FK: {err}"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_referral_capture_names_a_new_booking_and_not_a_v2_one() {
    let db = before_m1().await;
    let v2 = v2_bookings(&db).await;
    run_m1(&db).await.expect("M1 applies");
    let new = new_booking(&db).await;
    let c = customer(&db).await;

    db.exec(&capture_sql(&c, &new)).await;
    let err = refused(&db, &capture_sql(&c, &v2[1])).await;
    assert!(
        err.contains("referral_attributions_booking_id_fkey"),
        "a v2 id breaks the re-pointed FK: {err}"
    );
    db.finish().await;
}

#[tokio::test]
async fn m1_refuses_while_a_queue_ticket_names_a_booking_and_changes_nothing() {
    let db = before_m1().await;
    let v2 = v2_bookings(&db).await;
    db.exec(&ticket_sql(&v2[1], 1)).await;

    let err = run_m1(&db).await.expect_err("M1 refuses");
    assert!(
        err.contains("P0001"),
        "M1's own guard RAISEs (SQLSTATE P0001), not a constraint error: {err}"
    );
    assert!(
        has_column(&db, "bookings", "member_id").await && !exists(&db, "v2_bookings").await,
        "nothing renamed"
    );
    assert!(!exists(&db, "booking_slots").await, "nothing created");
    db.finish().await;
}

#[tokio::test]
async fn m1_refuses_while_a_referral_capture_names_a_booking_and_changes_nothing() {
    let db = before_m1().await;
    let v2 = v2_bookings(&db).await;
    let c = customer(&db).await;
    db.exec(&capture_sql(&c, &v2[1])).await;

    let err = run_m1(&db).await.expect_err("M1 refuses");
    assert!(
        err.contains("P0001"),
        "M1's own guard RAISEs (SQLSTATE P0001), not a constraint error: {err}"
    );
    assert!(
        has_column(&db, "bookings", "member_id").await && !exists(&db, "v2_bookings").await,
        "nothing renamed"
    );
    assert!(!exists(&db, "booking_slots").await, "nothing created");
    db.finish().await;
}

// ── item 4: down ────────────────────────────────────────────────────────────

#[tokio::test]
async fn down_refuses_while_a_queue_ticket_or_a_capture_names_a_new_booking() {
    for what in ["ticket", "capture"] {
        let db = after_m1().await;
        let new = new_booking(&db).await;
        if what == "ticket" {
            db.exec(&ticket_sql(&new, 1)).await;
        } else {
            let c = customer(&db).await;
            db.exec(&capture_sql(&c, &new)).await;
        }
        let err = run_down(&db).await.expect_err("down refuses");
        assert!(
            err.contains("P0001"),
            "{what}: down's own guard RAISEs (SQLSTATE P0001), not a constraint error: {err}"
        );
        assert!(
            exists(&db, "v2_bookings").await && exists(&db, "booking_slots").await,
            "{what}: nothing undone"
        );
        db.finish().await;
    }
}

#[tokio::test]
async fn down_restores_v2s_bookings_when_nothing_names_a_new_booking() {
    let db = before_m1().await;
    let v2 = v2_bookings(&db).await;
    let before = db
        .text("SELECT string_agg(row_to_json(b)::text, '|' ORDER BY b.id) FROM bookings b")
        .await;
    run_m1(&db).await.expect("M1 applies");
    new_booking(&db).await;

    run_down(&db).await.expect("down applies");
    for table in [
        "v2_bookings",
        "booking_policies",
        "booking_slot_rules",
        "branch_closures",
        "booking_slots",
        "booking_sequences",
        "booking_events",
    ] {
        assert!(!exists(&db, table).await, "{table} is gone");
    }
    assert!(
        has_column(&db, "bookings", "member_id").await,
        "bookings is v2's again"
    );
    assert_eq!(
        db.text("SELECT string_agg(row_to_json(b)::text, '|' ORDER BY b.id) FROM bookings b")
            .await,
        before,
        "v2's rows are back as they were"
    );
    // the queue's FK points at v2's table again
    db.exec(&ticket_sql(&v2[1], 1)).await;
    db.finish().await;
}

// ── the new schema's CHECKs ─────────────────────────────────────────────────

#[tokio::test]
async fn a_slot_cannot_hold_more_than_its_capacity() {
    let db = after_m1().await;
    let s = slot(&db, ORG_1, MAIN, 2).await;
    db.exec(&format!(
        "UPDATE booking_slots SET reserved = 2 WHERE id = '{s}'"
    ))
    .await;
    refused(
        &db,
        &format!("UPDATE booking_slots SET reserved = 3 WHERE id = '{s}'"),
    )
    .await;
    db.finish().await;
}

#[tokio::test]
async fn a_completed_booking_names_its_transaction_and_only_one_booking_does() {
    let db = after_m1().await;
    let c = customer(&db).await;
    let s = slot(&db, ORG_1, MAIN, 4).await;
    refused(
        &db,
        &booking_sql(
            &format!("BK-{}", Uuid::new_v4()),
            ORG_1,
            MAIN,
            &c,
            s,
            "COMPLETED",
            "DESK",
            None,
            None,
        ),
    )
    .await;

    let (t, _) = exchange(&db, &c, false).await;
    db.exec(&booking_sql(
        &format!("BK-{}", Uuid::new_v4()),
        ORG_1,
        MAIN,
        &c,
        s,
        "COMPLETED",
        "DESK",
        Some(t),
        None,
    ))
    .await;
    let err = refused(
        &db,
        &booking_sql(
            &format!("BK-{}", Uuid::new_v4()),
            ORG_1,
            MAIN,
            &c,
            s,
            "COMPLETED",
            "DESK",
            Some(t),
            None,
        ),
    )
    .await;
    assert!(
        err.contains("bookings_one_per_transaction"),
        "a second booking of one transaction: {err}"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_booking_cannot_take_another_branchs_slot() {
    let db = after_m1().await;
    db.branch("B2").await;
    let c = customer(&db).await;
    let other = slot(&db, ORG_1, "B2", 4).await;
    let err = refused(
        &db,
        &booking_sql(
            &format!("BK-{}", Uuid::new_v4()),
            ORG_1,
            MAIN,
            &c,
            other,
            "CONFIRMED",
            "DESK",
            None,
            None,
        ),
    )
    .await;
    assert!(err.contains("foreign key"), "the slot is B2's: {err}");
    db.finish().await;
}

/// Lane controller's ruling 3 on #1377: `(source_channel = 'PARTNER') <= (partner_attribution_id IS NOT NULL)`.
#[tokio::test]
async fn a_partner_booking_needs_its_partner_attribution() {
    let db = after_m1().await;
    let c = customer(&db).await;
    let s = slot(&db, ORG_1, MAIN, 4).await;
    refused(
        &db,
        &booking_sql(
            &format!("BK-{}", Uuid::new_v4()),
            ORG_1,
            MAIN,
            &c,
            s,
            "CONFIRMED",
            "PARTNER",
            None,
            None,
        ),
    )
    .await;
    db.exec(&booking_sql(
        &format!("BK-{}", Uuid::new_v4()),
        ORG_1,
        MAIN,
        &c,
        s,
        "CONFIRMED",
        "PARTNER",
        None,
        Some(Uuid::new_v4()),
    ))
    .await;
    db.finish().await;
}

#[tokio::test]
async fn booking_events_are_append_only() {
    let db = after_m1().await;
    let b = new_booking(&db).await;
    db.exec(&format!(
        "INSERT INTO booking_events (id, organization_id, booking_id, kind, to_status, actor_id, at) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{b}', 'CREATE', 'CONFIRMED', 'test', now())"
    ))
    .await;
    refused(&db, "UPDATE booking_events SET reason = 'changed'").await;
    refused(&db, "DELETE FROM booking_events").await;
    assert_eq!(db.count("SELECT count(*) FROM booking_events").await, 1);
    db.finish().await;
}
