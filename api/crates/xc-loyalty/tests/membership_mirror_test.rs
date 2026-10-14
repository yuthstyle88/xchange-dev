//! Ticket LY-4 (#1402, design `docs/design/blueprint/membership-points.md` D-8, §4.1, §4.5, §8 M1, §9.3 row
//! LY-4) — the tier mirror: migration M1, `PgMembershipMirror` and `MembershipSyncWorker::tick`, against a
//! fake feed (§9.1: "or a fake feed in tests"). Written before the implementation by the ticket author
//! (claude/89bd61d9, on the lane controller's order). Do not edit, delete or weaken; add tests in a file
//! of your own.
//!
//! The trait shapes are the ones proposed on #1402 (issuecomment-6117468724). The "unknown customer ref"
//! row of §9.3 is read as the phone-keyed design has it (#407 L-9, D-10; same comment, Q2): an item whose
//! `phoneKey` is not a phone is skipped and the rest of the page applies.
//!
//! Uses LY-1's harness (`tests/common/mod.rs`): a schema per test with `api/migrations/` applied. Needs
//! `TEST_DATABASE_URL`; nothing skips.

mod common;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use common::{ORG, TestDb, unique};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::AssertSqlSafe;
use uuid::Uuid;
use xc_kernel::pii::{IdentityType, LookupHasher, PiiKeys};
use xc_loyalty::application::sync_worker::{MembershipSyncWorker, TickOutcome};
use xc_loyalty::domain::membership_feed::{MembershipChange, MembershipFeed, MembershipPage};
use xc_loyalty::domain::membership_mirror::{MembershipMirror, MirroredTier};
use xc_loyalty::infrastructure::pg_membership_mirror::PgMembershipMirror;

const SUFFIX: &str = "_customer_tier_mirror.sql";
/// `main`'s newest migration when these tests were written.
const MAINS_NEWEST_AT_WRITING: i64 = 20261112110000;
const THAI: &str = "+66812345678";
const FOREIGN: &str = "+447700900123";

// ── the hasher customer_identities uses ─────────────────────────────────────

fn keys() -> PiiKeys {
    PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("a test key")
}

fn hasher() -> LookupHasher {
    LookupHasher::new(&keys())
}

/// What `customer_identities.lookup_hash` holds for this E.164 phone.
fn phone_hash(e164: &str) -> Vec<u8> {
    hasher().hash(IdentityType::Phone, e164).to_vec()
}

fn org() -> Uuid {
    Uuid::from_str(ORG).expect("ORG is a uuid")
}

fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, 3, 0, 0)
        .single()
        .expect("a valid instant")
}

// ── seeds ───────────────────────────────────────────────────────────────────

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn count(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// A LIVE customer of `organization`; answers its id.
async fn customer(db: &TestDb, organization: Uuid) -> String {
    let id = unique("CU");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2, $1, 'Test Customer', 'LIVE', now())",
    )
    .bind(&id)
    .bind(organization)
    .execute(&db.pool)
    .await
    .expect("insert a customer");
    id
}

/// A PHONE identity of `customer` with the lookup hash of `e164`.
async fn phone(
    db: &TestDb,
    organization: Uuid,
    customer: &str,
    e164: &str,
    verified: bool,
    status: &str,
) {
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
             lookup_hash, verified_at, status) \
         VALUES ($1, $2, $3, 'PHONE', '\\x00', 'test-1', $4, CASE WHEN $5 THEN now() END, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(organization)
    .bind(customer)
    .bind(phone_hash(e164))
    .bind(verified)
    .bind(status)
    .execute(&db.pool)
    .await
    .expect("insert a PHONE identity");
}

/// A customer of `organization` whose ACTIVE verified PHONE is `e164`.
async fn member(db: &TestDb, organization: Uuid, e164: &str) -> String {
    let c = customer(db, organization).await;
    phone(db, organization, &c, e164, true, "ACTIVE").await;
    c
}

async fn second_organisation(db: &TestDb) -> Uuid {
    let id = Uuid::new_v4();
    exec(
        db,
        &format!(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ('{id}', '{code}', NULL, 'Asia/Bangkok', now())",
            code = unique("ORG2")
        ),
    )
    .await;
    id
}

fn item(phone_key: &str, tier: &str, version: i32, seq: i64) -> MembershipChange {
    MembershipChange {
        phone_key: phone_key.to_owned(),
        customer_ref: Some(Uuid::new_v4()),
        tier_code: tier.to_owned(),
        tier_rule_id: Uuid::from_u128(0x7000 + u128::from(version.unsigned_abs())),
        tier_rule_version: version,
        effective_at: at(1),
        effective_until: Some(at(28)),
        seq,
    }
}

fn page(items: Vec<MembershipChange>) -> MembershipPage {
    let next = items.iter().map(|i| i.seq).max().unwrap_or(0);
    MembershipPage {
        changed: true,
        items,
        next,
    }
}

fn mirror(db: &TestDb) -> PgMembershipMirror {
    PgMembershipMirror::new(db.pool.clone(), hasher())
}

async fn apply(db: &TestDb, organization: Uuid, p: &MembershipPage) {
    let m = mirror(db);
    let mut tx = db.begin().await;
    m.apply_in(&mut tx, organization, p)
        .await
        .expect("the page applies");
    tx.commit().await.expect("commit");
}

async fn tier(db: &TestDb, organization: Uuid, customer: &str) -> Option<MirroredTier> {
    mirror(db)
        .tier_of(organization, customer)
        .await
        .expect("tier_of answers")
}

async fn tier_code(db: &TestDb, organization: Uuid, customer: &str) -> Option<String> {
    tier(db, organization, customer).await.map(|t| t.tier_code)
}

/// `(cursor, last_ok_at is set, last_error)` of `organization`; `None` when it has no cursor row.
async fn cursor(db: &TestDb, organization: Uuid) -> Option<(i64, bool, Option<String>)> {
    sqlx::query_as(
        "SELECT cursor, last_ok_at IS NOT NULL, last_error FROM loyalty_sync_cursor WHERE organization_id = $1",
    )
    .bind(organization)
    .fetch_optional(&db.pool)
    .await
    .expect("read loyalty_sync_cursor")
}

async fn mirror_text(db: &TestDb) -> String {
    sqlx::query_scalar(
        "SELECT coalesce(string_agg(to_jsonb(m)::text, '|' ORDER BY m.loyalty_seq), '') \
         FROM customer_tier_mirror m",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read customer_tier_mirror")
}

// ── the fake feed ───────────────────────────────────────────────────────────

/// Answers the queued pages in order and records every `after` it is asked for.
#[derive(Default)]
struct FakeFeed {
    answers: Mutex<VecDeque<AppResult<MembershipPage>>>,
    asked: Mutex<Vec<i64>>,
}

impl FakeFeed {
    fn with(answers: Vec<AppResult<MembershipPage>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<i64> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

impl MembershipFeed for FakeFeed {
    fn changes_after(&self, seq: i64) -> BoxFuture<'_, AppResult<MembershipPage>> {
        self.asked.lock().expect("not poisoned").push(seq);
        let answer = self
            .answers
            .lock()
            .expect("not poisoned")
            .pop_front()
            .expect("the test queued an answer for every tick");
        Box::pin(async move { answer })
    }
}

fn worker(db: &TestDb, organization: Uuid, feed: Arc<FakeFeed>) -> MembershipSyncWorker {
    MembershipSyncWorker::new(db.pool.clone(), organization, feed, Arc::new(mirror(db)))
}

fn unchanged(next: i64) -> MembershipPage {
    MembershipPage {
        changed: false,
        items: vec![],
        next,
    }
}

// ── M1: the file ────────────────────────────────────────────────────────────

fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// (version, text) of the one M1.
fn the_file() -> (i64, String) {
    let found: Vec<String> = std::fs::read_dir(migrations())
        .expect("read api/migrations")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .filter(|n| n.ends_with(SUFFIX))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one api/migrations/*{SUFFIX} (design §8 M1): {found:?}"
    );
    let version = found[0][..found[0].find('_').expect("<version>_")]
        .parse()
        .expect("a numeric version");
    let text = std::fs::read_to_string(migrations().join(&found[0])).expect("read M1");
    (version, text)
}

/// The `-- down:` section, uncommented.
fn down_sql(text: &str) -> String {
    let mut lines = text.lines();
    assert!(
        lines.any(|l| l.trim() == "-- down:"),
        "M1 carries a `-- down:` line"
    );
    let mut out = String::new();
    for l in lines {
        let t = l.trim_start();
        let body = t.strip_prefix("--").unwrap_or(t);
        out.push_str(body.strip_prefix(' ').unwrap_or(body));
        out.push('\n');
    }
    assert!(!out.trim().is_empty(), "the down section is not empty");
    out
}

#[test]
fn m1_sorts_after_mains_newest_and_carries_a_down_section() {
    let (version, text) = the_file();
    assert!(
        version > MAINS_NEWEST_AT_WRITING,
        "M1 ({version}) sorts after main's newest at writing ({MAINS_NEWEST_AT_WRITING})"
    );
    down_sql(&text);
}

#[tokio::test]
async fn m1_creates_the_two_tables_and_down_drops_only_them() {
    let db = TestDb::start().await;
    let c = member(&db, org(), THAI).await;
    apply(&db, org(), &page(vec![item("812345678", "GOLD", 1, 1)])).await;

    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM information_schema.table_constraints \
             WHERE table_schema = current_schema() AND table_name = 'customer_tier_mirror' \
               AND constraint_type = 'PRIMARY KEY'"
        )
        .await,
        1
    );
    let tables =
        "SELECT count(*) FROM information_schema.tables WHERE table_schema = current_schema()";
    let before = count(&db, tables).await;

    let (_, text) = the_file();
    sqlx::raw_sql(AssertSqlSafe(down_sql(&text)))
        .execute(&db.pool)
        .await
        .expect("down applies");
    assert_eq!(
        count(&db, tables).await,
        before - 2,
        "down drops two tables"
    );
    for gone in ["customer_tier_mirror", "loyalty_sync_cursor"] {
        assert_eq!(
            count(
                &db,
                &format!("SELECT count(*) FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = '{gone}'")
            )
            .await,
            0,
            "{gone} is gone"
        );
    }
    assert_eq!(
        count(
            &db,
            &format!("SELECT count(*) FROM customers WHERE id = '{c}'")
        )
        .await,
        1,
        "customers keep their rows"
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM customer_identities").await,
        1,
        "identities keep their rows"
    );
    db.finish().await;
}

// ── PgMembershipMirror ──────────────────────────────────────────────────────

/// §9.3 LY-4: Thai `812345678` ⇒ `+66812345678`; `+447700900123` stays E.164; both stored as that phone's
/// `lookup_hash`, and each customer's `tier_of` finds its own tier.
#[tokio::test]
async fn a_thai_and_a_foreign_phone_key_store_the_lookup_hash_of_their_e164_phone() {
    let db = TestDb::start().await;
    let thai = member(&db, org(), THAI).await;
    let foreign = member(&db, org(), FOREIGN).await;
    apply(
        &db,
        org(),
        &page(vec![
            item("812345678", "GOLD", 2, 11),
            item(FOREIGN, "SILVER", 3, 12),
        ]),
    )
    .await;

    let gold = tier(&db, org(), &thai)
        .await
        .expect("the Thai customer's tier");
    assert_eq!(gold.tier_code, "GOLD");
    assert_eq!(gold.tier_rule_version, 2);
    assert_eq!(gold.tier_rule_id, Uuid::from_u128(0x7002));
    assert_eq!(gold.effective_at, at(1));
    assert_eq!(gold.effective_until, Some(at(28)));
    assert_eq!(
        tier_code(&db, org(), &foreign).await.as_deref(),
        Some("SILVER")
    );

    let stored: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT phone_lookup_hash FROM customer_tier_mirror WHERE organization_id = $1 ORDER BY loyalty_seq",
    )
    .bind(org())
    .fetch_all(&db.pool)
    .await
    .expect("read the hashes");
    assert_eq!(stored, vec![phone_hash(THAI), phone_hash(FOREIGN)]);

    let text = mirror_text(&db).await;
    for plain in ["812345678", "447700900123"] {
        assert!(!text.contains(plain), "no plain phone is stored: {text}");
    }
    db.finish().await;
}

#[tokio::test]
async fn a_newer_seq_replaces_the_tier_and_an_older_or_repeated_one_never_does() {
    let db = TestDb::start().await;
    let c = member(&db, org(), THAI).await;

    apply(&db, org(), &page(vec![item("812345678", "GOLD", 1, 5)])).await;
    assert_eq!(tier_code(&db, org(), &c).await.as_deref(), Some("GOLD"));

    let newer = page(vec![item("812345678", "PLATINUM", 2, 7)]);
    apply(&db, org(), &newer).await;
    assert_eq!(tier_code(&db, org(), &c).await.as_deref(), Some("PLATINUM"));

    apply(&db, org(), &page(vec![item("812345678", "SILVER", 3, 6)])).await;
    assert_eq!(
        tier_code(&db, org(), &c).await.as_deref(),
        Some("PLATINUM"),
        "an older seq never overwrites a newer one"
    );
    apply(&db, org(), &newer).await;
    apply(&db, org(), &page(vec![item(THAI, "BRONZE", 4, 7)])).await;
    assert_eq!(
        tier_code(&db, org(), &c).await.as_deref(),
        Some("PLATINUM"),
        "the same seq again changes nothing"
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM customer_tier_mirror").await,
        1,
        "`812345678` and `+66812345678` are one member"
    );
    assert_eq!(
        count(&db, "SELECT max(loyalty_seq) FROM customer_tier_mirror").await,
        7
    );
    db.finish().await;
}

/// §4.1: `tier_of` joins the ACTIVE verified PHONE only, in the customer's own organisation.
#[tokio::test]
async fn tier_of_needs_an_active_verified_phone_of_the_same_organisation() {
    let db = TestDb::start().await;
    let other = second_organisation(&db).await;

    let unverified = customer(&db, org()).await;
    phone(&db, org(), &unverified, THAI, false, "ACTIVE").await;
    let revoked = customer(&db, org()).await;
    phone(&db, org(), &revoked, THAI, true, "REVOKED").await;
    let no_phone = customer(&db, org()).await;
    let elsewhere = member(&db, other, FOREIGN).await;

    // the member rows exist: THAI in ORG, FOREIGN in ORG only (not in the other organisation)
    apply(
        &db,
        org(),
        &page(vec![
            item("812345678", "GOLD", 1, 1),
            item(FOREIGN, "SILVER", 1, 2),
        ]),
    )
    .await;

    assert_eq!(
        tier_code(&db, org(), &unverified).await,
        None,
        "verified_at NULL"
    );
    assert_eq!(tier_code(&db, org(), &revoked).await, None, "REVOKED");
    assert_eq!(tier_code(&db, org(), &no_phone).await, None, "no PHONE");
    assert_eq!(
        tier_code(&db, other, &elsewhere).await,
        None,
        "the other organisation's mirror has no row for that phone"
    );

    // a member who later verifies finds the tier already mirrored
    let verified = member(&db, org(), FOREIGN).await;
    assert_eq!(
        tier_code(&db, org(), &verified).await.as_deref(),
        Some("SILVER")
    );
    db.finish().await;
}

/// §9.3's "unknown customer ref", read as #1402 issuecomment-6117468724 Q2: the member is the phone, so an
/// item whose `phoneKey` is not a phone is skipped and the rest of the page applies. `customerRef` is never
/// what decides.
#[tokio::test]
async fn an_item_whose_phone_key_is_not_a_phone_is_skipped_and_the_rest_applies() {
    let db = TestDb::start().await;
    let c = member(&db, org(), FOREIGN).await;
    let mut no_ref = item(FOREIGN, "SILVER", 1, 3);
    no_ref.customer_ref = None;
    apply(
        &db,
        org(),
        &page(vec![item("not-a-phone", "GOLD", 1, 2), no_ref]),
    )
    .await;

    assert_eq!(
        count(&db, "SELECT count(*) FROM customer_tier_mirror").await,
        1
    );
    assert_eq!(
        tier_code(&db, org(), &c).await.as_deref(),
        Some("SILVER"),
        "an item without a customerRef still applies"
    );
    db.finish().await;
}

// ── MembershipSyncWorker::tick ──────────────────────────────────────────────

#[tokio::test]
async fn a_tick_applies_one_page_and_moves_the_cursor_to_its_next() {
    let db = TestDb::start().await;
    let c = member(&db, org(), THAI).await;
    let mut first = page(vec![item("812345678", "GOLD", 1, 40)]);
    first.next = 42;
    let feed = FakeFeed::with(vec![
        Ok(first),
        Ok(page(vec![item("812345678", "PLATINUM", 2, 43)])),
    ]);
    let w = worker(&db, org(), feed.clone());

    assert_eq!(
        w.tick().await.expect("the first tick"),
        TickOutcome::Applied { items: 1 }
    );
    assert_eq!(tier_code(&db, org(), &c).await.as_deref(), Some("GOLD"));
    let (at_cursor, ok, error) = cursor(&db, org()).await.expect("a cursor row");
    assert_eq!(at_cursor, 42, "the cursor is the page's `next`");
    assert!(ok, "last_ok_at is set");
    assert_eq!(error, None);

    w.tick().await.expect("the second tick");
    assert_eq!(
        feed.asked(),
        vec![0, 42],
        "from cursor 0, then from the stored cursor"
    );
    assert_eq!(tier_code(&db, org(), &c).await.as_deref(), Some("PLATINUM"));
    assert_eq!(cursor(&db, org()).await.expect("a cursor row").0, 43);
    db.finish().await;
}

#[tokio::test]
async fn an_unchanged_answer_writes_nothing_and_keeps_the_cursor() {
    let db = TestDb::start().await;
    member(&db, org(), THAI).await;
    let feed = FakeFeed::with(vec![
        Ok(page(vec![item("812345678", "GOLD", 1, 9)])),
        Ok(unchanged(9)),
    ]);
    let w = worker(&db, org(), feed.clone());
    w.tick().await.expect("the first tick");
    let mirrored = mirror_text(&db).await;

    assert_eq!(
        w.tick().await.expect("the unchanged tick"),
        TickOutcome::Unchanged
    );
    assert_eq!(mirror_text(&db).await, mirrored, "the mirror is untouched");
    assert_eq!(cursor(&db, org()).await.expect("a cursor row").0, 9);
    assert_eq!(feed.asked(), vec![0, 9]);
    db.finish().await;
}

/// Writes the page through the real mirror, then fails on the second page it is given.
struct FailsSecondApply {
    inner: PgMembershipMirror,
    applied: Mutex<u32>,
}

impl MembershipMirror for FailsSecondApply {
    fn tier_of<'a>(
        &'a self,
        org: Uuid,
        customer: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<MirroredTier>>> {
        self.inner.tier_of(org, customer)
    }

    fn apply_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: Uuid,
        page: &'a MembershipPage,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            self.inner.apply_in(tx, org, page).await?;
            let mut n = self.applied.lock().expect("not poisoned");
            *n += 1;
            if *n == 2 {
                return Err(AppError::Internal("apply failed after writing".to_owned()));
            }
            Ok(())
        })
    }
}

/// §9.3: the cursor advances only after the page commits. The apply and the cursor share one transaction,
/// so a page whose apply fails after writing leaves neither its rows nor a moved cursor.
#[tokio::test]
async fn a_page_whose_apply_fails_leaves_no_row_and_no_moved_cursor() {
    let db = TestDb::start().await;
    let c = member(&db, org(), THAI).await;
    let w = MembershipSyncWorker::new(
        db.pool.clone(),
        org(),
        FakeFeed::with(vec![
            Ok(page(vec![item("812345678", "GOLD", 1, 3)])),
            Ok(page(vec![
                item("812345678", "PLATINUM", 2, 4),
                item(FOREIGN, "SILVER", 1, 5),
            ])),
        ]),
        Arc::new(FailsSecondApply {
            inner: mirror(&db),
            applied: Mutex::new(0),
        }),
    );
    w.tick().await.expect("the first tick");

    let _ = w.tick().await;
    assert_eq!(
        tier_code(&db, org(), &c).await.as_deref(),
        Some("GOLD"),
        "the failed page's rows are rolled back"
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM customer_tier_mirror").await,
        1
    );
    assert_eq!(cursor(&db, org()).await.expect("a cursor row").0, 3);
    db.finish().await;
}

/// §9: Loyalty down ⇒ the mirror keeps the last tier; `last_error` is set (≤ 500 characters, §4.1).
#[tokio::test]
async fn a_feed_failure_keeps_the_mirror_and_the_cursor_and_records_last_error() {
    let db = TestDb::start().await;
    let c = member(&db, org(), THAI).await;
    let long = format!("loyalty answered 503: {}", "x".repeat(2_000));
    let w = worker(
        &db,
        org(),
        FakeFeed::with(vec![
            Ok(page(vec![item("812345678", "GOLD", 1, 6)])),
            Err(AppError::Internal(long)),
        ]),
    );
    w.tick().await.expect("the first tick");
    let mirrored = mirror_text(&db).await;

    let _ = w.tick().await;
    assert_eq!(mirror_text(&db).await, mirrored, "the last tier is kept");
    assert_eq!(tier_code(&db, org(), &c).await.as_deref(), Some("GOLD"));
    let (at_cursor, _, error) = cursor(&db, org()).await.expect("a cursor row");
    assert_eq!(at_cursor, 6, "the cursor stays");
    let error = error.expect("last_error is set");
    assert!(!error.is_empty());
    assert!(
        error.chars().count() <= 500,
        "last_error is at most 500 characters, got {}",
        error.chars().count()
    );
    db.finish().await;
}

#[tokio::test]
async fn a_worker_moves_only_its_own_organisations_cursor() {
    let db = TestDb::start().await;
    let other = second_organisation(&db).await;
    let c = member(&db, other, THAI).await;
    let w = worker(
        &db,
        other,
        FakeFeed::with(vec![Ok(page(vec![item("812345678", "GOLD", 1, 8)]))]),
    );
    w.tick().await.expect("the tick");

    assert_eq!(cursor(&db, other).await.expect("its cursor").0, 8);
    assert_eq!(cursor(&db, org()).await.map(|c| c.0).unwrap_or(0), 0);
    assert_eq!(tier_code(&db, other, &c).await.as_deref(), Some("GOLD"));
    db.finish().await;
}

/// The cursor is read from the worker's own organisation too (rule-A check on #1402,
/// issuecomment-6117882511): ORG's cursor stands at 50, then the other organisation's worker asks from
/// its own position (0, then its own `next`), never from ORG's, and each cursor stays its own.
#[tokio::test]
async fn a_worker_reads_only_its_own_organisations_cursor() {
    let db = TestDb::start().await;
    let other = second_organisation(&db).await;
    let theirs = member(&db, other, THAI).await;

    let mut ours = page(vec![item(FOREIGN, "SILVER", 1, 48)]);
    ours.next = 50;
    worker(&db, org(), FakeFeed::with(vec![Ok(ours)]))
        .tick()
        .await
        .expect("ORG's tick");
    assert_eq!(cursor(&db, org()).await.expect("ORG's cursor").0, 50);

    let feed = FakeFeed::with(vec![
        Ok(page(vec![item("812345678", "GOLD", 1, 7)])),
        Ok(unchanged(7)),
    ]);
    let w = worker(&db, other, feed.clone());
    w.tick().await.expect("the other organisation's first tick");
    w.tick().await.expect("its second tick");

    assert_eq!(
        feed.asked(),
        vec![0, 7],
        "the other organisation asks from its own cursor, never from ORG's 50"
    );
    assert_eq!(
        tier_code(&db, other, &theirs).await.as_deref(),
        Some("GOLD"),
        "a member at a seq below ORG's cursor is mirrored for the other organisation"
    );
    assert_eq!(cursor(&db, other).await.expect("its cursor").0, 7);
    assert_eq!(cursor(&db, org()).await.expect("ORG's cursor").0, 50);
    db.finish().await;
}
