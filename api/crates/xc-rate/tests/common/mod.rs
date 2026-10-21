//! Written before the implementation (ticket T5a). Do not edit, delete or weaken.
//!
//! Shared by `rates_test.rs` and `branches_test.rs`: the database harness, the
//! v2 fixture, and a reference-rate source the tests control.
//!
//! The single `pub use` block below is every path of `xc-rate` the tests name.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. When the variable is absent every test
//! FAILS; none of them skips. Each `TestDb` is a schema of its own with
//! `api/migrations/` applied, so tests run in parallel and the migration is
//! exercised from an empty schema every time. SQL in the migration and in the
//! implementation must therefore name tables without a schema prefix.
//!
//! Re-expected for RE-7 #604 by the ticket author (claude/07d3765f): RE-7 removes
//! `RateService::save_rate` and drops `rates` (`docs/design/blueprint/rates.md` §8 migration 6,
//! §9). `play` no longer saves rates: a `saveRate` step is v2's direct save, whose replay is
//! retired with `rates_test.rs`'s (§9 row "`rates_test.rs` … the direct save → publications"); the
//! fixture keeps recording it. Two helpers are new: `TestDb::start_before_rates_drop` (every
//! migration older than RE-7's drop, for tests of what an earlier migration did to `rates`) and
//! `TestDb::publish` (a branch's next publication through the rate engine, draft → submit →
//! publish, where a test used to save a counter rate).
#![allow(dead_code)]

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate};
use xc_rate::application::approval::rate_publication;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};

pub use xc_rate::application::branch::service::BranchService;
pub use xc_rate::application::rate::service::RateService;
pub use xc_rate::domain::branch::commands::create_branch::CreateBranch;
pub use xc_rate::domain::branch::entities::branch::Branch;
pub use xc_rate::domain::rate::entities::counter_rate::CounterRate;
pub use xc_rate::domain::rate::entities::rate_change::RateChange;
pub use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(20);
/// How long something is watched to prove it has not happened.
pub const STAYS_OUT: Duration = Duration::from_millis(400);

pub const ACTOR: &str = "owner";
pub const MAIN: &str = "MAIN";

// The twelve sentences v2 answers with in this context (fixture key `sentences`).
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
pub const BAD_REQUEST_ID: &str = "รหัสคำขอไม่ถูกต้อง";
pub const BAD_NUMBER: &str = "จำนวนเงินหรือเรทไม่ถูกต้อง";
pub const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";
pub const SELL_BELOW_BUY: &str = "เรทร้านขายต้องไม่น้อยกว่าเรทร้านรับซื้อ";
pub const ABANDONED: &str = "คำขอนี้ถูกยกเลิกแล้ว กรุณาโหลดข้อมูลล่าสุด";
pub const PARTNER_PAIR: &str = "กรอกชื่อคู่ค้าและเรทต้นทุนให้ครบคู่";
pub const REQUEST_CHANGED: &str = "คำขอเดิมมีข้อมูลเปลี่ยน";
pub const EDITED_ELSEWHERE: &str = "เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน";
pub const NAME_TAKEN: &str = "ชื่อสาขานี้มีอยู่แล้ว";
pub const TOO_MANY_BRANCHES: &str = "รองรับสูงสุด 20 สาขา";
/// What the reference-rate source of v2 says when it cannot answer.
pub const SOURCE_DOWN: &str = "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท";

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// A request id in v2's shape (`[A-Za-z0-9-]{16,60}`), different on every call.
pub fn request_id() -> String {
    format!("req-{:012}-{:08}", std::process::id(), {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        SEQ.fetch_add(1, Ordering::Relaxed)
    })
}

pub fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

/// Await `fut`, failing the test when it takes longer than `limit`.
pub async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {limit:?}: {what}"),
    }
}

/// 2026-10-06 12:00:00 UTC plus `seconds`, with microseconds that are not whole
/// milliseconds: what a wall clock hands the service.
pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_288_000 + seconds, 123_456_000)
        .single()
        .expect("a valid instant")
}

/// The same instant with everything below the millisecond dropped.
pub fn millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .expect("a valid instant")
}

/// What `Date.prototype.toISOString()` prints: `2026-10-06T12:00:00.123Z`.
pub fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The sentence of a refusal. Panics on success and on any error that is not
/// `AppError::Domain` — a refusal reaches the screen as its sentence and nothing else.
pub fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(6);

/// The description sqlx gives RE-7's migration 6 (`<version>_drop_rates.sql`, §8).
const DROP_RATES: &str = "drop rates";

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    pub async fn start() -> Self {
        Self::start_with(|_| true).await
    }

    /// The same, with only the migrations older than RE-7's `drop_rates` (all of them where it
    /// does not exist yet): `rates` is there as every earlier migration left it.
    pub async fn start_before_rates_drop() -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let all = Migrator::new(dir.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", dir.display()));
        let drop = all
            .migrations
            .iter()
            .find(|m| m.description == DROP_RATES)
            .map_or(i64::MAX, |m| m.version);
        Self::start_with(move |version| version < drop).await
    }

    async fn start_with(keep: impl Fn(i64) -> bool) -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The xc-rate tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`. \
             Use a dedicated test database."
        );

        let schema = unique("t5");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let mut migrator = Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()));
        let kept: Vec<_> = migrator
            .migrations
            .iter()
            .filter(|m| keep(m.version))
            .cloned()
            .collect();
        migrator.migrations = std::borrow::Cow::Owned(kept);
        migrator
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        Self { pool, schema }
    }

    pub fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    pub fn branches(&self) -> BranchService {
        BranchService::new(self.branch_tx())
    }

    /// A rate service whose reference rates come from `source`.
    pub fn rates_with(&self, source: Arc<FakeSource>) -> RateService {
        RateService::new(self.branch_tx(), source)
    }

    /// A rate service with a reference-rate source that always answers.
    pub fn rates(&self) -> RateService {
        self.rates_with(FakeSource::answering())
    }

    /// Publish `lines` (`(currency, buy, sell)`, MANUAL, each rate per the currency's
    /// `unit_size`) as the next publication of `branch`, as the owner: draft → submit (an owner's
    /// draft is approved at once) → publish, the only way rates are set since RE-7 #604. Answers
    /// the new publication's id.
    pub async fn publish(&self, branch_id: &str, lines: &[(&str, &str, &str)]) -> String {
        let mut registry = ApprovalRegistry::new();
        rate_publication::register(&mut registry, self.pool.clone());
        let repository: Arc<dyn ApprovalRepository> =
            Arc::new(PgApprovalRepository::new(self.pool.clone()));
        let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(self.pool.clone()));
        let approvals = Arc::new(ApprovalService::new(registry, repository, audit));
        let drafts = RateDraftService::new(self.pool.clone(), approvals);
        let publisher = PublishService::new(self.pool.clone());
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
        let b = branch(branch_id);
        let draft_lines: Vec<DraftLine> = lines
            .iter()
            .map(|(code, buy, sell)| {
                let side = |t: &str| {
                    LineSide::Manual(Rate::parse(t).unwrap_or_else(|e| panic!("{t}: {e:?}")))
                };
                DraftLine::new(
                    CurrencyCode::parse(code).unwrap_or_else(|_| panic!("{code} is a currency")),
                    side(buy),
                    side(sell),
                    Some("published by the test".to_owned()),
                )
            })
            .collect();

        let mut tx = self.pool.begin().await.expect("begin");
        let draft = drafts
            .create(&mut tx, &b, &owner_user, draft_lines)
            .await
            .unwrap_or_else(|e| panic!("create the draft: {}", e.code()));
        tx.commit().await.expect("commit the draft");

        let mut tx = self.pool.begin().await.expect("begin");
        drafts
            .submit(&mut tx, &b, &owner_user, &draft.id, &unique("key"))
            .await
            .unwrap_or_else(|e| panic!("submit the draft: {}", e.code()));
        tx.commit().await.expect("commit the submit");

        let mut tx = self.pool.begin().await.expect("begin");
        let expected = self.current_version(&mut tx, branch_id).await;
        let published = publisher
            .publish(&mut tx, &b, &owner_user, &draft.id, expected)
            .await
            .unwrap_or_else(|e| panic!("publish the draft: {}", e.code()));
        tx.commit().await.expect("commit the publish");
        published.id.to_string()
    }

    async fn current_version(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch_id: &str,
    ) -> Option<i32> {
        sqlx::query_scalar(
            "SELECT p.version FROM branches b JOIN rate_publications p \
               ON p.id = b.current_publication_id WHERE b.id = $1",
        )
        .bind(branch_id)
        .fetch_optional(&mut **tx)
        .await
        .expect("read the current version")
    }

    /// `(currency, buy_rate, sell_rate)` of every line of `branch`'s current publication, by
    /// currency; empty before its first publication.
    pub async fn published_lines(&self, branch_id: &str) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "SELECT trim(l.currency_code), l.buy_rate::text, l.sell_rate::text \
               FROM branches b JOIN published_rate_lines l \
                 ON l.publication_id = b.current_publication_id \
              WHERE b.id = $1 ORDER BY l.currency_code",
        )
        .bind(branch_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the published lines of {branch_id}: {e}"))
    }

    /// Mark a request id as abandoned in a branch, as `adminAbandonRequest` will.
    pub async fn abandon(&self, branch_id: &str, request_id: &str) {
        sqlx::query(
            "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) \
             VALUES ($1, $2, now(), $3)",
        )
        .bind(branch_id)
        .bind(request_id)
        .bind(ACTOR)
        .execute(&self.pool)
        .await
        .expect("insert into abandoned_requests");
    }

    /// Add a branch behind the service's back.
    pub async fn seed_branch(&self, id: &str, name: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(&self.pool)
            .await
            .expect("insert into branches");
    }

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// Drop the schema. Reached only by a test that passed; a failed test
    /// leaves its schema behind for inspection.
    pub async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect to drop the test schema");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }
}

/// How the fake source behaves on its next calls.
enum Behaviour {
    Answer,
    Fail,
    /// Announce the call, then wait for a permit before answering.
    Hold {
        entered: Arc<Semaphore>,
        release: Arc<Semaphore>,
    },
}

/// A reference-rate source under the test's control. It counts its calls.
pub struct FakeSource {
    behaviour: Behaviour,
    calls: AtomicUsize,
}

impl FakeSource {
    pub fn answering() -> Arc<Self> {
        Arc::new(Self {
            behaviour: Behaviour::Answer,
            calls: AtomicUsize::new(0),
        })
    }

    /// Fails every call with [`SOURCE_DOWN`].
    pub fn failing() -> Arc<Self> {
        Arc::new(Self {
            behaviour: Behaviour::Fail,
            calls: AtomicUsize::new(0),
        })
    }

    /// Every call adds a permit to `entered` and then waits for one on `release`.
    pub fn holding() -> (Arc<Self>, Arc<Semaphore>, Arc<Semaphore>) {
        let entered = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let source = Arc::new(Self {
            behaviour: Behaviour::Hold {
                entered: entered.clone(),
                release: release.clone(),
            },
            calls: AtomicUsize::new(0),
        });
        (source, entered, release)
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// The row the source holds for one of the six currencies it knows.
    pub fn row(code: &str) -> Value {
        json!({
            "code": code, "date": "2026-10-05", "unit": if code == "JPY" { 100 } else { 1 },
            "buyingSight": 32.0125, "buyingTransfer": 32.1, "selling": 32.45,
            "original": { "buyingSight": "32.0125", "buyingTransfer": "32.1000", "selling": "32.4500" }
        })
    }

    pub fn everything() -> ReferenceRates {
        ReferenceRates {
            source: "https://example.test/reference".to_owned(),
            fetched_at: "2026-10-06T11:59:00.000Z".to_owned(),
            kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_owned(),
            rows: ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"]
                .into_iter()
                .map(Self::row)
                .collect(),
        }
    }

    /// What must be stored beside a rate row of `code`: the source's answer with
    /// only that currency's rows.
    pub fn stored_for(code: &str) -> Value {
        let known = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"].contains(&code);
        json!({
            "source": "https://example.test/reference",
            "fetchedAt": "2026-10-06T11:59:00.000Z",
            "kind": "DAILY_COMMERCIAL_BANK_AVERAGE",
            "rows": if known { vec![Self::row(code)] } else { vec![] },
        })
    }
}

impl RateSource for FakeSource {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.behaviour {
                Behaviour::Answer => Ok(Self::everything()),
                Behaviour::Fail => Err(AppError::Domain(SOURCE_DOWN.to_owned())),
                Behaviour::Hold { entered, release } => {
                    entered.add_permits(1);
                    release.acquire().await.expect("never closed").forget();
                    Ok(Self::everything())
                }
            }
        })
    }
}

// ── the v2 fixture ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Fixture {
    #[serde(rename = "mainBranch")]
    pub main_branch: FxBranch,
    pub sentences: Vec<String>,
    pub currencies: Vec<FxCurrency>,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
pub struct FxCurrency {
    pub code: String,
    pub name: String,
    pub symbol: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct FxBranch {
    pub id: String,
    pub name: String,
    #[serde(rename = "requestId", default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub op: String,
    pub branch: String,
    pub request_id: String,
    /// What v2 answered: `None` when it accepted the step.
    pub v2_error: Option<String>,
    // saveRate
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub buy: String,
    #[serde(default)]
    pub sell: String,
    #[serde(default)]
    pub previous_updated: String,
    pub partner_name: Option<String>,
    /// Absent, `null` and `""` all mean "no partner cost was sent".
    pub partner_cost: Option<String>,
    pub after: Option<After>,
    // abandon
    pub resolution: Option<String>,
    // createBranch
    #[serde(default)]
    pub name: String,
    pub created_id: Option<String>,
    pub branches_after: Option<Vec<FxBranch>>,
}

#[derive(Debug, Deserialize)]
pub struct After {
    pub rate: Option<FxRate>,
    pub history: Vec<FxChange>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FxRate {
    pub buy: Option<String>,
    pub sell: Option<String>,
    pub request_id: String,
    pub partner_name: String,
    pub partner_cost: Option<String>,
    pub has_updated: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FxChange {
    pub code: String,
    pub buy: String,
    pub sell: String,
    pub actor: String,
    pub partner_name: String,
    pub partner_cost: Option<String>,
}

pub fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/v2_rate_cases.json"))
        .expect("tests/fixtures/v2_rate_cases.json parses")
}

/// A time v2's fixture generator stands for with `@stale`: never a row's own.
pub const STALE: &str = "2001-01-01T00:00:00.000Z";

pub fn as_fx_rate(rate: &CounterRate) -> FxRate {
    FxRate {
        buy: rate.buy.map(|r| r.to_string()),
        sell: rate.sell.map(|r| r.to_string()),
        request_id: rate.request_id.clone().unwrap_or_default(),
        partner_name: rate.partner_name.clone(),
        partner_cost: rate.partner_cost.map(|r| r.to_string()),
        has_updated: rate.updated_at.is_some(),
    }
}

pub fn as_fx_change(change: &RateChange) -> FxChange {
    FxChange {
        code: change.code.as_str().to_owned(),
        buy: change.buy.to_string(),
        sell: change.sell.to_string(),
        actor: change.updated_by.clone(),
        partner_name: change.partner_name.clone(),
        partner_cost: change.partner_cost.map(|r| r.to_string()),
    }
}

pub fn as_fx_branch(b: &Branch) -> FxBranch {
    FxBranch {
        id: b.id.as_str().to_owned(),
        name: b.name.clone(),
        request_id: b.request_id.clone(),
    }
}

/// Play one fixture scenario against a fresh schema and return every place where
/// the services answered differently from v2.
pub async fn play(scenario: &Scenario) -> Vec<String> {
    let db = TestDb::start().await;
    let branches = db.branches();
    let mut wrong = Vec::new();

    for (i, step) in scenario.steps.iter().enumerate() {
        let here = format!("'{}' step {} ({})", scenario.name, i + 1, step.op);
        let now = at(i as i64);
        let branch_id = branch(&step.branch);
        match step.op.as_str() {
            "abandon" => {
                // v2 records an abandonment only for a request that was never applied.
                if step.resolution.as_deref() == Some("CANCELLED") {
                    db.abandon(&step.branch, &step.request_id).await;
                }
            }
            "saveRate" => panic!(
                "{here}: v2's direct save is not replayed since RE-7 #604 removed \
                 `RateService::save_rate` (rates.md §9); rates come from publications"
            ),
            "createBranch" => {
                let cmd = CreateBranch {
                    request_id: step.request_id.clone(),
                    name: step.name.clone(),
                    from_branch: branch_id.clone(),
                };
                let got = branches.create(cmd, ACTOR, now).await;
                let created = got.as_ref().ok().map(|b| b.id.as_str().to_owned());
                compare_outcome(&mut wrong, &here, &step.v2_error, got.map(|_| ()));
                if step.v2_error.is_none() && created != step.created_id {
                    wrong.push(format!(
                        "{here}: created {created:?}, v2 created {:?}",
                        step.created_id
                    ));
                }
                let expected = step
                    .branches_after
                    .as_ref()
                    .expect("createBranch steps list the branches");
                match branches.list().await {
                    Ok(list) => {
                        let got: Vec<FxBranch> = list.iter().map(as_fx_branch).collect();
                        if &got != expected {
                            wrong
                                .push(format!("{here}: branches are {got:?}, v2 has {expected:?}"));
                        }
                    }
                    Err(e) => wrong.push(format!("{here}: list() failed: {e:?}")),
                }
            }
            other => {
                panic!("{here}: the fixture has an operation this test does not know: {other}")
            }
        }
    }

    if wrong.is_empty() {
        db.finish().await;
    }
    wrong
}

fn compare_outcome(
    wrong: &mut Vec<String>,
    here: &str,
    v2_error: &Option<String>,
    got: AppResult<()>,
) {
    match (v2_error, got) {
        (None, Ok(())) => {}
        (Some(want), Err(AppError::Domain(sentence))) if &sentence == want => {}
        (want, got) => wrong.push(format!(
            "{here}: v2 answered {want:?}, the service answered {got:?}"
        )),
    }
}

/// Play every scenario `keep` selects, several at a time, and fail with the full
/// list of differences.
pub async fn play_all(keep: impl Fn(&Scenario) -> bool) {
    use futures::stream::{self, StreamExt};
    let fixture = fixture();
    let chosen: Vec<&Scenario> = fixture.scenarios.iter().filter(|s| keep(s)).collect();
    assert!(
        !chosen.is_empty(),
        "the filter selected no scenario of the fixture"
    );
    let played = chosen.len();
    let wrong: Vec<String> = stream::iter(chosen)
        .map(play)
        .buffer_unordered(6)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect();
    assert!(
        wrong.is_empty(),
        "{} difference(s) from v2 in {played} scenario(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// The kind of a scenario: `"saveRate"` when it saves a rate anywhere, else `"createBranch"`.
pub fn kind(scenario: &Scenario) -> &'static str {
    if scenario.steps.iter().any(|s| s.op == "saveRate") {
        "saveRate"
    } else {
        "createBranch"
    }
}
