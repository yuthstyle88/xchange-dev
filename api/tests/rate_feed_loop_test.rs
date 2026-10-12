//! Rate feed F10a — the BoT refresh loop (`docs/design/rate-feed-connector.md` §11; owner's
//! rulings R-12, R-12b, R-12c, R-12d on #986).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `quote_common/mod.rs`). Every test works in a schema of its own.
//!
//! What this pins:
//! - `xchange::infrastructure::rate_feed::feed_loop::{ReferenceFetch, BranchRefresh,
//!   RefreshOutcome, RateFeedConfig, RateFeedLoop, TickReport, RATE_FEED_LOCK_KEY}` and
//!   `xchange::infrastructure::rate_feed::branch_refresh::DraftBranchRefresh` (§11.2).
//! - `DraftBranchRefresh::refresh` reuses the draft path (§11.3): copy the published draft, submit
//!   it as SYSTEM, publish when in bounds; out of bounds waits for the owner, once per batch; no
//!   publication or a publication with no draft ⇒ `NothingToCopy`, nothing written.
//! - `RateFeedLoop::tick` (§11.4): one advisory lock, one fetch, one transaction per
//!   `BOT_DRAFT` branch in id order; a fetch error touches no branch; one branch's error is
//!   rolled back and does not stop the others; a failed tick does not leave the lock held.
//! - `RateFeedConfig::from_env` (§11.2): `RATE_FEED_INTERVAL_SECS`, default 10800; a value that
//!   is not a whole number > 0 ⇒ the default.
//! - `RateFeedLoop::run` ticks once at start and returns when `stop` turns true (§11.4).

mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quote_common::*;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xchange::infrastructure::rate_feed::branch_refresh::DraftBranchRefresh;
use xchange::infrastructure::rate_feed::feed_loop::{
    BranchRefresh, RATE_FEED_LOCK_KEY, RateFeedConfig, RateFeedLoop, ReferenceFetch,
    RefreshOutcome, TickReport,
};
use xchange::presentation::http::v3::commands::make_approval_service;

type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `RATE_FEED_LOCK_KEY` is an advisory lock of the whole database, not of a test's schema. Tests
/// that tick or hold it take this guard first, so that one test's tick is never skipped because
/// another test holds the lock.
static FEED_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------------------------------------------------------------------------------------------
// Seeding
// ---------------------------------------------------------------------------------------------

/// A BoT reference batch, fetched now, with one line `(currency, unit_size, buy, sell)` each.
async fn batch(db: &TestDb, lines: &[(&str, &str, &str, &str)]) -> Uuid {
    let id = Uuid::new_v4();
    let checksum = unique("sum");
    db.exec(&format!(
        "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES ('{id}', 'BOT', current_date, now(), '{checksum}', 'COMPLETE')"
    ))
    .await;
    for (code, unit, buy, sell) in lines {
        db.exec(&format!(
            "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
             VALUES (gen_random_uuid(), '{id}', '{code}', 'BOT', {unit}, {buy}, {sell}, '{{}}'::jsonb)"
        ))
        .await;
    }
    // Two batches written in one test must not share `fetched_at`, or "newest" is a coin toss.
    tokio::time::sleep(Duration::from_millis(5)).await;
    id
}

/// A policy version of `branch`: `adjust_pct` on both sides, and a USD guard `[min, max]` on both.
async fn policy(db: &TestDb, branch: &str, adjust_pct: &str, min: &str, max: &str) {
    db.exec(&format!(
        "INSERT INTO rate_policy_versions (id, organization_id, branch_id, version, \
             buy_adjust_pct, sell_adjust_pct, guards, max_reference_age_hours, on_stale, \
             stale_limit_thb, valid_from, valid_to, approved_by, reason, source_version) \
         SELECT gen_random_uuid(), b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_policy_versions WHERE branch_id = b.id), 0) + 1, \
             {adjust_pct}, {adjust_pct}, \
             '{{\"USD\": {{\"buy\": {{\"min_pct\": \"{min}\", \"max_pct\": \"{max}\"}}, \
                          \"sell\": {{\"min_pct\": \"{min}\", \"max_pct\": \"{max}\"}}}}}}'::jsonb, \
             24, NULL, NULL, now() - interval '1 day', NULL, 'test', 'test', NULL \
         FROM branches b WHERE b.id = '{branch}'"
    ))
    .await;
}

fn drafts(pool: &PgPool) -> Arc<RateDraftService> {
    Arc::new(RateDraftService::new(
        pool.clone(),
        make_approval_service(pool.clone()),
    ))
}

fn refresher(pool: &PgPool) -> DraftBranchRefresh {
    DraftBranchRefresh::new(drafts(pool), Arc::new(PublishService::new(pool.clone())))
}

/// The owner publishes an all-`AUTO` USD draft on `branch` through the draft path, as today.
async fn owner_publishes_auto_usd(db: &TestDb, branch: &str) {
    // OR-7.2 #1021: `create`, `submit` and `publish` take the owner's B1-mapped user, EXECUTIVE + ADMIN organisation-wide.
    let owner_user = xc_auth::domain::access::Actor::new(
        "owner",
        "00000000-0000-0000-0000-000000000001",
        vec![
            xc_auth::domain::access::ScopedRole::new(
                xc_auth::domain::access::Role::Executive,
                None,
                chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                None,
            ),
            xc_auth::domain::access::ScopedRole::new(
                xc_auth::domain::access::Role::Admin,
                None,
                chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                None,
            ),
        ],
        None,
    );
    let b = BranchId::new(branch);
    let svc = drafts(&db.pool);
    let line = DraftLine::new(
        CurrencyCode::parse("USD").expect("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    );
    let mut tx = db.pool.begin().await.expect("begin");
    let draft = svc
        .create(&mut tx, &b, &owner_user, vec![line])
        .await
        .unwrap_or_else(|e| panic!("create: {}", e.code()));
    svc.submit(&mut tx, &b, &owner_user, &draft.id, &unique("owner-key"))
        .await
        .unwrap_or_else(|e| panic!("submit: {}", e.code()));
    let expected = current_version(&mut tx, branch).await;
    PublishService::new(db.pool.clone())
        .publish(&mut tx, &b, &owner_user, &draft.id, expected)
        .await
        .unwrap_or_else(|e| panic!("publish: {}", e.code()));
    tx.commit().await.expect("commit");
}

async fn current_version(tx: &mut Transaction<'_, Postgres>, branch: &str) -> Option<i32> {
    sqlx::query_scalar(
        "SELECT p.version FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch)
    .fetch_optional(&mut **tx)
    .await
    .expect("read the current version")
}

async fn usd_buy(db: &TestDb, branch: &str) -> String {
    db.text(&format!(
        "SELECT l.buy_rate::text FROM branches b JOIN published_rate_lines l \
           ON l.publication_id = b.current_publication_id \
         WHERE b.id = '{branch}' AND l.currency_code = 'USD'"
    ))
    .await
    .expect("a USD buy rate")
}

async fn version(db: &TestDb, branch: &str) -> i64 {
    db.count(&format!(
        "SELECT COALESCE((SELECT p.version::bigint FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = '{branch}'), 0)"
    ))
    .await
}

async fn draft_count(db: &TestDb, branch: &str) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM rate_drafts WHERE branch_id = '{branch}'"
    ))
    .await
}

async fn refresh_once(db: &TestDb, branch: &str, batch: Uuid) -> RefreshOutcome {
    let r = refresher(&db.pool);
    let b = BranchId::new(branch);
    let mut tx = db.pool.begin().await.expect("begin");
    let out = r
        .refresh(&mut tx, &b, batch)
        .await
        .unwrap_or_else(|e| panic!("refresh: {}", e.code()));
    tx.commit().await.expect("commit");
    out
}

fn dec(text: &str) -> rust_decimal::Decimal {
    text.parse().expect("a decimal")
}

// ---------------------------------------------------------------------------------------------
// DraftBranchRefresh (§11.3)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_in_bounds_refresh_publishes_the_new_batch_with_the_policys_adjust_pct() {
    let db = TestDb::start().await;
    let branch = unique("f10in");
    db.branch(&branch).await;
    policy(&db, &branch, "1", "-5", "5").await;
    batch(&db, &[("USD", "1", "32.00", "33.00")]).await;
    owner_publishes_auto_usd(&db, &branch).await;
    assert_eq!(version(&db, &branch).await, 1);

    let newer = batch(&db, &[("USD", "1", "34.00", "35.00")]).await;
    let out = refresh_once(&db, &branch, newer).await;

    assert!(
        matches!(out, RefreshOutcome::Published(_)),
        "in bounds publishes, got {out:?}"
    );
    assert_eq!(version(&db, &branch).await, 2, "a new publication");
    // R-12d: 34.00 × (1 + 1/100) = 34.34
    assert_eq!(dec(&usd_buy(&db, &branch).await), dec("34.34"));
    db.finish().await;
}

#[tokio::test]
async fn the_refresh_publishes_as_the_system_actor_not_as_the_owner() {
    let db = TestDb::start().await;
    let branch = unique("f10sys");
    db.branch(&branch).await;
    policy(&db, &branch, "0", "-5", "5").await;
    batch(&db, &[("USD", "1", "32.00", "33.00")]).await;
    owner_publishes_auto_usd(&db, &branch).await;
    let newer = batch(&db, &[("USD", "1", "34.00", "35.00")]).await;
    refresh_once(&db, &branch, newer).await;

    let by = db
        .text(&format!(
            "SELECT d.created_by FROM branches b JOIN rate_publications p \
               ON p.id = b.current_publication_id JOIN rate_drafts d ON d.id = p.draft_id \
             WHERE b.id = '{branch}'"
        ))
        .await
        .expect("the published draft's author");
    assert_eq!(
        by, "SYSTEM:rate-feed:BOT",
        "§11.3: the SYSTEM actor writes the draft"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_out_of_bounds_refresh_waits_for_the_owner_and_asks_once_per_batch() {
    let db = TestDb::start().await;
    let branch = unique("f10out");
    db.branch(&branch).await;
    // adjust 3 % lies outside the [0, 1] guard: every AUTO side is out of bounds.
    policy(&db, &branch, "3", "0", "1").await;
    batch(&db, &[("USD", "1", "32.00", "33.00")]).await;
    owner_publishes_auto_usd(&db, &branch).await; // the owner decides it themselves
    let buy_before = usd_buy(&db, &branch).await;

    let newer = batch(&db, &[("USD", "1", "34.00", "35.00")]).await;
    let first = refresh_once(&db, &branch, newer).await;
    assert!(
        matches!(first, RefreshOutcome::AwaitingOwner(_)),
        "out of bounds waits for the owner, got {first:?}"
    );
    assert_eq!(version(&db, &branch).await, 1, "nothing published");
    assert_eq!(usd_buy(&db, &branch).await, buy_before, "the price stays");
    let status = db
        .text(&format!(
            "SELECT status FROM rate_drafts WHERE branch_id = '{branch}' \
               AND created_by = 'SYSTEM:rate-feed:BOT'"
        ))
        .await;
    assert_eq!(status.as_deref(), Some("PENDING_APPROVAL"));

    let drafts_after_first = draft_count(&db, &branch).await;
    let second = refresh_once(&db, &branch, newer).await;
    assert_eq!(
        second,
        RefreshOutcome::UpToDate,
        "the same batch is not asked again"
    );
    assert_eq!(draft_count(&db, &branch).await, drafts_after_first);
    db.finish().await;
}

#[tokio::test]
async fn a_batch_already_published_is_up_to_date_and_writes_nothing() {
    let db = TestDb::start().await;
    let branch = unique("f10same");
    db.branch(&branch).await;
    policy(&db, &branch, "0", "-5", "5").await;
    batch(&db, &[("USD", "1", "32.00", "33.00")]).await;
    owner_publishes_auto_usd(&db, &branch).await;
    let newer = batch(&db, &[("USD", "1", "34.00", "35.00")]).await;
    assert!(matches!(
        refresh_once(&db, &branch, newer).await,
        RefreshOutcome::Published(_)
    ));
    let drafts_before = draft_count(&db, &branch).await;

    assert_eq!(
        refresh_once(&db, &branch, newer).await,
        RefreshOutcome::UpToDate
    );
    assert_eq!(version(&db, &branch).await, 2);
    assert_eq!(draft_count(&db, &branch).await, drafts_before);
    db.finish().await;
}

#[tokio::test]
async fn a_branch_with_no_publication_has_nothing_to_copy() {
    let db = TestDb::start().await;
    let branch = unique("f10none");
    db.branch(&branch).await;
    policy(&db, &branch, "0", "-5", "5").await;
    let b = batch(&db, &[("USD", "1", "32.00", "33.00")]).await;

    assert_eq!(
        refresh_once(&db, &branch, b).await,
        RefreshOutcome::NothingToCopy
    );
    assert_eq!(draft_count(&db, &branch).await, 0, "nothing written");
    assert_eq!(version(&db, &branch).await, 0);
    db.finish().await;
}

#[tokio::test]
async fn a_publication_with_no_draft_has_nothing_to_copy() {
    let db = TestDb::start().await;
    let branch = unique("f10nodraft");
    db.branch(&branch).await;
    policy(&db, &branch, "0", "-5", "5").await;
    // `TestDb::publish` writes a publication with `draft_id = NULL`, as a board publication has.
    db.publish(
        &branch,
        &[Line {
            code: "USD",
            unit_size: "1",
            buy: Some("32.00"),
            sell: Some("33.00"),
            stale_under: None,
        }],
    )
    .await;
    let b = batch(&db, &[("USD", "1", "34.00", "35.00")]).await;

    assert_eq!(
        refresh_once(&db, &branch, b).await,
        RefreshOutcome::NothingToCopy
    );
    assert_eq!(draft_count(&db, &branch).await, 0, "nothing written");
    assert_eq!(version(&db, &branch).await, 1);
    db.finish().await;
}

// ---------------------------------------------------------------------------------------------
// RateFeedLoop (§11.4) with a fake fetch and a fake refresh
// ---------------------------------------------------------------------------------------------

struct FakeFetch {
    calls: Mutex<u32>,
    fail: bool,
    batch: Uuid,
}

impl FakeFetch {
    fn ok(batch: Uuid) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(0),
            fail: false,
            batch,
        })
    }
    fn failing() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(0),
            fail: true,
            batch: Uuid::nil(),
        })
    }
    fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }
}

impl ReferenceFetch for FakeFetch {
    fn fetch_and_write<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
    ) -> Fut<'a, Result<Uuid, RateEngineError>> {
        Box::pin(async move {
            *self.calls.lock().unwrap() += 1;
            if self.fail {
                Err(RateEngineError::RateUnavailable("BoT is down".into()))
            } else {
                Ok(self.batch)
            }
        })
    }
}

/// Records `(branch, batch)` per call. A branch in `fail_on` first writes a marker row through
/// the transaction it was given, then errors, so the test can see that the write is rolled back.
struct FakeRefresh {
    calls: Mutex<Vec<(String, Uuid)>>,
    fail_on: Vec<String>,
}

impl FakeRefresh {
    fn new(fail_on: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            fail_on: fail_on.iter().map(|s| s.to_string()).collect(),
        })
    }
    fn calls(&self) -> Vec<(String, Uuid)> {
        self.calls.lock().unwrap().clone()
    }
}

impl BranchRefresh for FakeRefresh {
    fn refresh<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        batch: Uuid,
    ) -> Fut<'a, Result<RefreshOutcome, RateEngineError>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((branch.as_str().to_owned(), batch));
            if self.fail_on.iter().any(|b| b == branch.as_str()) {
                sqlx::query(
                    "UPDATE branches SET name = 'touched by a failed refresh' WHERE id = $1",
                )
                .bind(branch.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;
                Err(RateEngineError::Internal("refresh failed".into()))
            } else {
                Ok(RefreshOutcome::UpToDate)
            }
        })
    }
}

fn loop_with(
    pool: &PgPool,
    fetch: Arc<FakeFetch>,
    refresh: Arc<FakeRefresh>,
    interval: Duration,
) -> RateFeedLoop {
    RateFeedLoop::new(pool.clone(), fetch, refresh, RateFeedConfig { interval })
}

/// The `BOT_DRAFT` branches this test created, in the order `tick` must visit them.
fn mine(calls: &[(String, Uuid)], prefix: &str) -> Vec<String> {
    calls
        .iter()
        .filter(|(b, _)| b.starts_with(prefix))
        .map(|(b, _)| b.clone())
        .collect()
}

#[tokio::test]
async fn a_tick_fetches_once_and_refreshes_every_bot_draft_branch_in_id_order() {
    let _serial = FEED_LOCK.lock().await;
    let db = TestDb::start().await;
    let prefix = unique("f10tick");
    let (a, b, c) = (
        format!("{prefix}_a"),
        format!("{prefix}_b"),
        format!("{prefix}_c"),
    );
    // created out of order on purpose
    db.branch(&c).await;
    db.branch(&a).await;
    db.branch(&b).await;
    db.exec(&format!(
        "UPDATE branches SET price_source = 'RATE_BOARD' WHERE id = '{b}'"
    ))
    .await;

    let batch_id = Uuid::new_v4();
    let fetch = FakeFetch::ok(batch_id);
    let refresh = FakeRefresh::new(&[]);
    let lp = loop_with(
        &db.pool,
        fetch.clone(),
        refresh.clone(),
        Duration::from_secs(3600),
    );
    let report: TickReport = lp
        .tick()
        .await
        .unwrap_or_else(|e| panic!("tick: {}", e.code()));

    assert!(!report.skipped);
    assert_eq!(fetch.calls(), 1, "one fetch per tick");
    assert_eq!(report.batch, Some(batch_id));
    assert_eq!(report.fetch_error, None);
    let calls = refresh.calls();
    assert_eq!(
        mine(&calls, &prefix),
        vec![a.clone(), c.clone()],
        "BOT_DRAFT branches only, ordered by id; a RATE_BOARD branch is never touched"
    );
    assert!(
        calls.iter().all(|(_, bt)| *bt == batch_id),
        "every branch gets the tick's batch"
    );
    assert!(
        report
            .refreshed
            .iter()
            .any(|(br, out)| br == &a && *out == RefreshOutcome::UpToDate)
    );
    assert!(report.failed.is_empty());
    db.finish().await;
}

#[tokio::test]
async fn a_fetch_error_touches_no_branch() {
    let _serial = FEED_LOCK.lock().await;
    let db = TestDb::start().await;
    let prefix = unique("f10ferr");
    db.branch(&format!("{prefix}_a")).await;
    let fetch = FakeFetch::failing();
    let refresh = FakeRefresh::new(&[]);
    let lp = loop_with(
        &db.pool,
        fetch.clone(),
        refresh.clone(),
        Duration::from_secs(3600),
    );
    let report = lp
        .tick()
        .await
        .unwrap_or_else(|e| panic!("tick: {}", e.code()));

    assert!(!report.skipped);
    assert_eq!(fetch.calls(), 1);
    assert_eq!(report.batch, None);
    assert!(
        report
            .fetch_error
            .as_deref()
            .is_some_and(|m| m.contains("BoT is down")),
        "the fetch error is reported, got {:?}",
        report.fetch_error
    );
    assert!(
        refresh.calls().is_empty(),
        "no branch is refreshed after a failed fetch"
    );
    db.finish().await;
}

#[tokio::test]
async fn one_branchs_failure_is_rolled_back_and_does_not_stop_the_others() {
    let _serial = FEED_LOCK.lock().await;
    let db = TestDb::start().await;
    let prefix = unique("f10bad");
    let (a, b, c) = (
        format!("{prefix}_a"),
        format!("{prefix}_b"),
        format!("{prefix}_c"),
    );
    for br in [&a, &b, &c] {
        db.branch(br).await;
    }
    let fetch = FakeFetch::ok(Uuid::new_v4());
    let refresh = FakeRefresh::new(&[b.as_str()]);
    let lp = loop_with(&db.pool, fetch, refresh.clone(), Duration::from_secs(3600));
    let report = lp
        .tick()
        .await
        .unwrap_or_else(|e| panic!("tick: {}", e.code()));

    assert_eq!(
        mine(&refresh.calls(), &prefix),
        vec![a.clone(), b.clone(), c.clone()]
    );
    assert!(
        report
            .failed
            .iter()
            .any(|(br, msg)| br == &b && msg.contains("refresh failed")),
        "the failing branch is reported, got {:?}",
        report.failed
    );
    assert!(report.refreshed.iter().any(|(br, _)| br == &a));
    assert!(report.refreshed.iter().any(|(br, _)| br == &c));
    let name = db
        .text(&format!("SELECT name FROM branches WHERE id = '{b}'"))
        .await;
    assert_eq!(
        name.as_deref(),
        Some(b.as_str()),
        "the failing branch's writes are rolled back"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_tick_skips_when_another_holds_the_lock() {
    let _serial = FEED_LOCK.lock().await;
    let db = TestDb::start().await;
    db.branch(&unique("f10lock")).await;
    let mut holder = db.pool.acquire().await.expect("a connection");
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(RATE_FEED_LOCK_KEY)
        .execute(&mut *holder)
        .await
        .expect("hold the lock");

    let fetch = FakeFetch::ok(Uuid::new_v4());
    let refresh = FakeRefresh::new(&[]);
    let lp = loop_with(
        &db.pool,
        fetch.clone(),
        refresh.clone(),
        Duration::from_secs(3600),
    );
    let report = lp
        .tick()
        .await
        .unwrap_or_else(|e| panic!("tick: {}", e.code()));
    assert!(report.skipped, "another holder ⇒ skipped");
    assert_eq!(fetch.calls(), 0, "nothing runs while skipped");
    assert!(refresh.calls().is_empty());

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(RATE_FEED_LOCK_KEY)
        .execute(&mut *holder)
        .await
        .expect("release the lock");
    drop(holder);
    db.finish().await;
}

#[tokio::test]
async fn a_failed_tick_does_not_leave_the_lock_held() {
    let _serial = FEED_LOCK.lock().await;
    let db = TestDb::start().await;
    db.branch(&unique("f10relock")).await;
    let fetch = FakeFetch::failing();
    let refresh = FakeRefresh::new(&[]);
    let lp = loop_with(&db.pool, fetch.clone(), refresh, Duration::from_secs(3600));
    let first = lp
        .tick()
        .await
        .unwrap_or_else(|e| panic!("tick: {}", e.code()));
    let second = lp
        .tick()
        .await
        .unwrap_or_else(|e| panic!("tick: {}", e.code()));
    assert!(
        !first.skipped && !second.skipped,
        "the second tick is not locked out"
    );
    assert_eq!(fetch.calls(), 2);

    let free: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(RATE_FEED_LOCK_KEY)
        .fetch_one(&db.pool)
        .await
        .expect("try the lock");
    assert!(free, "no session holds the lock after the ticks");
    db.finish().await;
}

#[tokio::test]
async fn run_ticks_once_at_start_and_returns_when_stopped() {
    let _serial = FEED_LOCK.lock().await;
    let db = TestDb::start().await;
    db.branch(&unique("f10run")).await;
    let fetch = FakeFetch::ok(Uuid::new_v4());
    let refresh = FakeRefresh::new(&[]);
    let lp = Arc::new(loop_with(
        &db.pool,
        fetch.clone(),
        refresh,
        Duration::from_secs(3600),
    ));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let handle = tokio::spawn(lp.run(stop_rx));

    let started = tokio::time::timeout(Duration::from_secs(20), async {
        while fetch.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        started.is_ok(),
        "run ticks once at start, before the first interval"
    );

    stop_tx.send(true).expect("signal stop");
    let stopped = tokio::time::timeout(Duration::from_secs(10), handle).await;
    assert!(stopped.is_ok(), "run returns when stop turns true");
    assert_eq!(
        fetch.calls(),
        1,
        "no second tick inside the hour-long interval"
    );
    db.finish().await;
}

// ---------------------------------------------------------------------------------------------
// RateFeedConfig (§11.2, R-12b)
// ---------------------------------------------------------------------------------------------

/// One test, so that no other test in this binary reads the variable while it changes.
#[test]
fn the_interval_defaults_to_three_hours_and_is_a_setting() {
    let key = "RATE_FEED_INTERVAL_SECS";
    // SAFETY: no other test in this binary reads or writes this variable.
    unsafe { std::env::remove_var(key) };
    assert_eq!(
        RateFeedConfig::from_env().interval,
        Duration::from_secs(10_800)
    );
    for (value, want) in [
        ("60", 60u64),
        ("21600", 21_600),
        ("0", 10_800),
        ("-5", 10_800),
        ("abc", 10_800),
        ("", 10_800),
        ("1.5", 10_800),
    ] {
        unsafe { std::env::set_var(key, value) };
        assert_eq!(
            RateFeedConfig::from_env().interval,
            Duration::from_secs(want),
            "RATE_FEED_INTERVAL_SECS={value:?}"
        );
    }
    unsafe { std::env::remove_var(key) };
}
