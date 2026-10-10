//! Written before the implementation (ticket T11d). Do not edit, delete or weaken.
//!
//! `adminAbandonRequest`: the screen gives up waiting for the answer to a write
//! and asks what became of its request id. Recorded from v2 in
//! `tests/fixtures/snapshot/branches.json` #35–#39 and
//! `tests/fixtures/rpc/engine_actions.json` #145–#152.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. When the variable is absent every test
//! FAILS; none of them skips. Each `TestDb` is a schema of its own with
//! `api/migrations/` applied.
//!
//! The writes that apply a request are made through the services on main
//! (`BranchService`, `RateService`, `ShiftService`), never by hand: what
//! "applied" means is whatever those services store.

mod compat_common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use compat_common::{Fixture, Step};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::{Barrier, Semaphore, oneshot};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::branch::commands::create_branch::CreateBranch;
use xc_rate::domain::rate::commands::save_rate::SaveRate;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::commands::close_shift::CloseShift;
use xc_shift::domain::shift::commands::move_stock::MoveStock;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::entities::shift::Shift;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::application::compat::abandon_request::{AbandonRequest, RequestResolution};

use RequestResolution::{Applied, Cancelled};

/// Upper bound for anything that must finish.
const MUST_FINISH: Duration = Duration::from_secs(30);
/// How long something is watched to prove it has not happened.
const STAYS_OUT: Duration = Duration::from_millis(400);

const ACTOR: &str = "owner";
const MAIN: &str = "MAIN";

// The sentences v2 answers with.
const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
const BAD_REQUEST_ID: &str = "รหัสคำขอไม่ถูกต้อง";
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
const ABANDONED: &str = "คำขอนี้ถูกยกเลิกแล้ว กรุณาโหลดข้อมูลล่าสุด";
const NOT_YOUR_ROLE: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้";

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A name no other test, test binary or run on this server has used.
fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// A request id in v2's shape (`[A-Za-z0-9-]{16,60}`), different on every call.
fn request_id() -> String {
    format!(
        "req-{:012}-{:08}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

/// 2026-10-06 12:00:00 UTC plus `seconds`, with microseconds that are not whole
/// milliseconds: what a wall clock hands the use case.
fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_288_000 + seconds, 123_456_000)
        .single()
        .expect("a valid instant")
}

/// The same instant with everything below the millisecond dropped.
fn millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .expect("a valid instant")
}

async fn within<T>(what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(MUST_FINISH, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {MUST_FINISH:?}: {what}"),
    }
}

/// The sentence of a refusal. Panics on success and on any error that is not
/// `AppError::Domain` — a refusal reaches the screen as its sentence and nothing else.
fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

/// A reference-rate source that always answers, with no rows.
struct QuietSource;

impl RateSource for QuietSource {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = AppResult<ReferenceRates>> + Send + '_>> {
        Box::pin(async {
            Ok(ReferenceRates {
                source: "test".to_string(),
                fetched_at: "2026-10-06T00:00:00.000Z".to_string(),
                kind: "test".to_string(),
                rows: Vec::new(),
            })
        })
    }
}

/// branch_id, request_id, at, actor
type AbandonedRow = (String, String, DateTime<Utc>, String);

struct TestDb {
    pool: PgPool,
    schema: String,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(6);

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The abandon-request tests need a real PostgreSQL and do not skip. Point it \
                 at an existing database whose name ends in `_test`, e.g.\n\
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

        let schema = unique("t11d");
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

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        for opener in ["owner", "owner-1", "test"] {
            seed_opener(&pool, opener).await;
        }

        Self { pool, schema }
    }

    fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    /// The use case under test.
    fn abandoning(&self) -> AbandonRequest {
        AbandonRequest::new(self.branch_tx())
    }

    fn branches(&self) -> BranchService {
        BranchService::new(self.branch_tx())
    }

    fn rates(&self) -> RateService {
        RateService::new(self.branch_tx(), Arc::new(QuietSource))
    }

    fn shifts(&self) -> ShiftService {
        ShiftService::new(
            Arc::new(PgShiftRepository::new(self.pool.clone())),
            self.branch_tx(),
        )
    }

    /// `adminAbandonRequest` with `{requestId: id}` in `branch_id`, at `at(seconds)`.
    async fn abandon(
        &self,
        branch_id: &str,
        id: &str,
        seconds: i64,
    ) -> AppResult<RequestResolution> {
        within(
            "abandon",
            self.abandoning().abandon(
                &branch(branch_id),
                &json!({ "requestId": id }),
                ACTOR,
                at(seconds),
            ),
        )
        .await
    }

    /// Add a branch behind the service's back (no request id).
    async fn seed_branch(&self, id: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("Branch {id}"))
            .execute(&self.pool)
            .await
            .expect("insert into branches");
    }

    async fn save_rate(&self, branch_id: &str, id: &str) -> AppResult<()> {
        within(
            "save a rate",
            self.rates()
                .save_rate(&branch(branch_id), usd_rate(id), ACTOR, at(0)),
        )
        .await
    }

    async fn open_shift(&self, branch_id: &str, id: &str) -> AppResult<Shift> {
        within(
            "open a shift",
            self.shifts()
                .open(&branch(branch_id), opening(id), ACTOR, at(0)),
        )
        .await
    }

    /// Every row of `abandoned_requests`, ordered by branch and request id.
    async fn rows(&self) -> Vec<AbandonedRow> {
        sqlx::query_as(
            "SELECT branch_id, request_id, at, actor FROM abandoned_requests \
             ORDER BY branch_id, request_id",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read abandoned_requests")
    }

    /// `(branch_id, request_id)` of every row, in the same order.
    async fn keys(&self) -> Vec<(String, String)> {
        self.rows()
            .await
            .into_iter()
            .map(|(b, r, _, _)| (b, r))
            .collect()
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// Drop the schema. Reached only by a test that passed; a failed test
    /// leaves its schema behind to be looked at.
    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

fn usd_rate(id: &str) -> SaveRate {
    SaveRate {
        request_id: id.to_string(),
        code: "USD".to_string(),
        buy: "32.1".to_string(),
        sell: "32.6".to_string(),
        partner_name: String::new(),
        partner_cost: None,
        previous_updated: String::new(),
    }
}

fn opening(id: &str) -> OpenShift {
    OpenShift::from_payload(&json!({ "requestId": id, "opening": { "USD": 10 } }))
        .expect("a valid opening")
}

fn pair(branch_id: &str, id: &str) -> (String, String) {
    (branch_id.to_string(), id.to_string())
}

// ---------------------------------------------------------------------------
// The recorded steps
// ---------------------------------------------------------------------------

/// A payload field as the text the rate service takes: text as it is, a number
/// as JSON prints it, nothing as `""`.
fn field(payload: &Value, key: &str) -> String {
    match payload.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        None | Some(Value::Null) => String::new(),
        Some(other) => panic!("`{key}` is neither text nor a number: {other}"),
    }
}

fn payload_of(step: &Step) -> &Value {
    step.payload
        .as_ref()
        .unwrap_or_else(|| panic!("step {} has no payload", step.n))
}

/// The branch a step is sent to: `payload.branchId`, or the main one.
fn branch_of(step: &Step) -> BranchId {
    match payload_of(step).get("branchId") {
        Some(Value::String(id)) => branch(id),
        _ => branch(MAIN),
    }
}

fn instant_of(step: &Step) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&step.at)
        .unwrap_or_else(|e| panic!("step {}: `at` is not a time: {e}", step.n))
        .with_timezone(&Utc)
}

/// What v2 answered: `Ok("APPLIED" | "CANCELLED")` or `Err(<the sentence>)`.
fn recorded(step: &Step) -> Result<String, String> {
    if let Some(sentence) = step.error() {
        assert_eq!(step.status, Some(400), "step {}", step.n);
        return Err(sentence.to_string());
    }
    assert_eq!(step.status, Some(200), "step {}", step.n);
    match step.json().get("requestResolution") {
        Some(Value::String(r)) => Ok(r.clone()),
        other => panic!("step {}: no `requestResolution`: {other:?}", step.n),
    }
}

/// Applies one recorded step to v3 and returns what the screen would be told.
async fn replay(db: &TestDb, step: &Step) -> Result<String, String> {
    let payload = payload_of(step);
    let branch = branch_of(step);
    let now = instant_of(step);
    let answer = match step.action.as_deref() {
        Some("adminAbandonRequest") => db
            .abandoning()
            .abandon(&branch, payload, ACTOR, now)
            .await
            .map(|r| r.as_str().to_string()),
        Some("adminSaveRate") => {
            let cmd = SaveRate {
                request_id: field(payload, "requestId"),
                code: field(payload, "code"),
                buy: field(payload, "buy"),
                sell: field(payload, "sell"),
                partner_name: field(payload, "partnerName"),
                partner_cost: None,
                previous_updated: field(payload, "previousUpdated"),
            };
            db.rates()
                .save_rate(&branch, cmd, ACTOR, now)
                .await
                .map(|()| "saved".to_string())
        }
        Some("adminOpenShift") => match OpenShift::from_payload(payload) {
            Ok(cmd) => db
                .shifts()
                .open(&branch, cmd, ACTOR, now)
                .await
                .map(|_| "saved".to_string()),
            Err(e) => Err(e.into()),
        },
        Some("adminCreateBranch") => db
            .branches()
            .create(
                CreateBranch {
                    request_id: field(payload, "requestId"),
                    name: field(payload, "name"),
                    from_branch: branch.clone(),
                },
                ACTOR,
                now,
            )
            .await
            .map(|_| "saved".to_string()),
        other => panic!("step {}: no replay for {other:?}", step.n),
    };
    match answer {
        Ok(text) => Ok(text),
        Err(AppError::Domain(sentence)) => Err(sentence),
        Err(other) => panic!("step {}: not a refusal: {other:?}", step.n),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn branches_fixture_last_five_steps_answer_as_v2_did() {
    let db = TestDb::start().await;
    let fixture = Fixture::load("snapshot/branches.json");
    let steps = fixture.steps();
    assert_eq!(steps.len(), 39, "the file this test was written against");

    // #2 created the airport; its request id is the one #39 asks about.
    let created = fixture.step(2);
    assert_eq!(created.action.as_deref(), Some("adminCreateBranch"));
    assert_eq!(replay(&db, created).await, Ok("saved".to_string()));
    let airport = "B00000000-0000-4000-8000-0000000a0002";
    assert!(
        db.branches().require(&branch(airport)).await.is_ok(),
        "the branch the recorded steps name"
    );

    let last_five = &steps[34..];
    let actions: Vec<&str> = last_five
        .iter()
        .filter_map(|s| s.action.as_deref())
        .collect();
    assert_eq!(
        actions,
        [
            "adminAbandonRequest",
            "adminAbandonRequest",
            "adminOpenShift",
            "adminAbandonRequest",
            "adminAbandonRequest",
        ]
    );
    let mut answers = Vec::new();
    for step in last_five {
        let expected = recorded(step);
        let actual = replay(&db, step).await;
        assert_eq!(actual, expected, "snapshot/branches.json#{}", step.n);
        answers.push(expected);
    }
    // what the file holds, spelled out so that a reader of this test sees it
    assert_eq!(
        answers,
        [
            Ok("CANCELLED".to_string()),
            Ok("CANCELLED".to_string()),
            Err(ABANDONED.to_string()),
            Ok("CANCELLED".to_string()),
            Ok("APPLIED".to_string()),
        ]
    );

    // one row per branch for the id given up; none for the applied one; the row
    // of MAIN is the one #35 wrote, not #36
    let given_up = "00000000-0000-4000-8000-0000000a0007";
    assert_eq!(
        db.rows().await,
        vec![
            (
                airport.to_string(),
                given_up.to_string(),
                instant_of(fixture.step(38)),
                ACTOR.to_string()
            ),
            (
                MAIN.to_string(),
                given_up.to_string(),
                instant_of(fixture.step(35)),
                ACTOR.to_string()
            ),
        ]
    );
    assert_eq!(db.count("shifts").await, 0);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_actions_fixture_145_to_152_answer_as_v2_did() {
    let db = TestDb::start().await;
    let fixture = Fixture::load("rpc/engine_actions.json");

    // The two writes the recorded steps ask about: #23 saved a rate, #40 opened a shift.
    for n in [23, 40] {
        let step = fixture.step(n);
        assert_eq!(step.status, Some(200), "rpc/engine_actions.json#{n}");
        assert_eq!(replay(&db, step).await, Ok("saved".to_string()), "#{n}");
    }

    let steps: Vec<&Step> = (145..=152).map(|n| fixture.step(n)).collect();
    let actions: Vec<&str> = steps.iter().filter_map(|s| s.action.as_deref()).collect();
    assert_eq!(
        actions,
        [
            "adminAbandonRequest",
            "adminAbandonRequest",
            "adminAbandonRequest",
            "adminSaveRate",
            "adminOpenShift",
            "adminAbandonRequest",
            "adminAbandonRequest",
            "adminAbandonRequest",
        ]
    );

    let mut answers = Vec::new();
    for step in &steps[..7] {
        assert_eq!(step.sent_as, "owner", "#{}", step.n);
        let expected = recorded(step);
        let actual = replay(&db, step).await;
        assert_eq!(actual, expected, "rpc/engine_actions.json#{}", step.n);
        answers.push(expected);
    }
    assert_eq!(
        answers,
        [
            Err(BAD_TEXT.to_string()),
            Err(BAD_REQUEST_ID.to_string()),
            Ok("CANCELLED".to_string()),
            Err(ABANDONED.to_string()),
            Err(ABANDONED.to_string()),
            Ok("APPLIED".to_string()),
            Ok("APPLIED".to_string()),
        ]
    );

    // #152 is an employee: refused by the role table before any use case runs.
    // The use case takes no role, so nothing is replayed — the step is pinned
    // here so that a change of the file is seen.
    let by_front = steps[7];
    assert_ne!(by_front.sent_as, "owner");
    assert_eq!(recorded(by_front), Err(NOT_YOUR_ROLE.to_string()));

    assert_eq!(
        db.keys().await,
        vec![pair(MAIN, "00000000-0000-4000-8000-0000000a003f")]
    );
    assert_eq!(db.count("rates").await, 1);
    assert_eq!(db.count("shifts").await, 1);
    db.finish().await;
}

// ---------------------------------------------------------------------------
// Reading the request id
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_request_id_that_is_missing_or_too_long_is_refused_as_text() {
    let db = TestDb::start().await;
    let long = "a".repeat(61);
    let payloads = [
        json!({}),
        json!({ "requestId": null }),
        json!({ "requestId": "" }),
        json!({ "requestId": "   " }),
        json!({ "requestId": long }),
        json!({ "requestId": true }),
        json!({ "requestId": ["0123456789abcdef"] }),
        json!({ "requestId": { "id": "0123456789abcdef" } }),
        json!(null),
        json!([]),
        json!("0123456789abcdef"),
    ];
    for payload in &payloads {
        let result = db
            .abandoning()
            .abandon(&branch(MAIN), payload, ACTOR, at(0))
            .await;
        assert_eq!(refusal(result), BAD_TEXT, "payload {payload}");
    }
    assert_eq!(db.rows().await, vec![]);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_id_of_the_wrong_shape_is_refused() {
    let db = TestDb::start().await;
    for id in [
        "short",
        "0123456789abcde",      // 15
        "0123456789abcde_",     // 16 with an underscore
        "0123456789 abcdef",    // a space inside
        "0123456789abcdef.",    // a dot
        "กกกกกกกกกกกกกกกกกกกก", // 20 letters, none of them ASCII
    ] {
        let result = db.abandon(MAIN, id, 0).await;
        assert_eq!(refusal(result), BAD_REQUEST_ID, "id {id:?}");
    }
    assert_eq!(db.rows().await, vec![]);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sixteen_and_sixty_characters_are_accepted_and_the_id_is_trimmed() {
    let db = TestDb::start().await;
    let sixteen = "0123456789abcdeF";
    let sixty = "A-".repeat(30);
    assert_eq!(db.abandon(MAIN, sixteen, 0).await.unwrap(), Cancelled);
    assert_eq!(db.abandon(MAIN, &sixty, 0).await.unwrap(), Cancelled);

    // spaces around the id are not part of it
    let padded = request_id();
    assert_eq!(
        db.abandon(MAIN, &format!("  {padded}\t"), 0).await.unwrap(),
        Cancelled
    );
    // a number is read as its digits
    let result = db
        .abandoning()
        .abandon(
            &branch(MAIN),
            &json!({ "requestId": 1234567890123456_u64 }),
            ACTOR,
            at(0),
        )
        .await;
    assert_eq!(result.unwrap(), Cancelled);

    let mut expected = vec![
        pair(MAIN, sixteen),
        pair(MAIN, &sixty),
        pair(MAIN, &padded),
        pair(MAIN, "1234567890123456"),
    ];
    expected.sort();
    assert_eq!(db.keys().await, expected);
    assert_eq!(refusal(db.open_shift(MAIN, &padded).await), ABANDONED);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_that_does_not_exist_is_refused_and_nothing_is_written() {
    let db = TestDb::start().await;
    let id = request_id();
    assert_eq!(refusal(db.abandon("NOWHERE", &id, 0).await), NO_SUCH_BRANCH);
    // the branch is looked at before the request id, as every write on main does
    let result = db
        .abandoning()
        .abandon(&branch("NOWHERE"), &json!({}), ACTOR, at(0))
        .await;
    assert_eq!(refusal(result), NO_SUCH_BRANCH);
    assert_eq!(db.rows().await, vec![]);
    assert_eq!(db.count("branch_versions").await, 0);
    db.finish().await;
}

// ---------------------------------------------------------------------------
// CANCELLED
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn never_applied_is_cancelled_and_the_row_says_who_and_when() {
    let db = TestDb::start().await;
    let id = request_id();
    assert_eq!(db.abandon(MAIN, &id, 7).await.unwrap(), Cancelled);
    // the time is kept to the millisecond, as every time of this code base
    assert_eq!(
        db.rows().await,
        vec![(
            MAIN.to_string(),
            id.clone(),
            millis(at(7)),
            ACTOR.to_string()
        )]
    );
    assert_ne!(
        millis(at(7)),
        at(7),
        "the clock of this test has microseconds"
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn twice_is_the_same_answer_and_one_row_the_first() {
    let db = TestDb::start().await;
    let id = request_id();
    assert_eq!(db.abandon(MAIN, &id, 1).await.unwrap(), Cancelled);
    let again = within(
        "abandon again",
        db.abandoning().abandon(
            &branch(MAIN),
            &json!({ "requestId": id }),
            "somebody-else",
            at(99),
        ),
    )
    .await;
    assert_eq!(again.unwrap(), Cancelled);
    assert_eq!(db.abandon(MAIN, &id, 200).await.unwrap(), Cancelled);
    assert_eq!(
        db.rows().await,
        vec![(MAIN.to_string(), id, millis(at(1)), ACTOR.to_string())]
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_request_is_refused_by_every_write_of_the_branch() {
    let db = TestDb::start().await;
    // a shift to move stock in and to close, opened with another request
    let shift = db.open_shift(MAIN, &request_id()).await.unwrap();
    db.seed_branch("B2").await;

    let id = request_id();
    assert_eq!(db.abandon(MAIN, &id, 0).await.unwrap(), Cancelled);
    assert_eq!(db.abandon("B2", &id, 0).await.unwrap(), Cancelled);

    assert_eq!(refusal(db.save_rate(MAIN, &id).await), ABANDONED);
    assert_eq!(refusal(db.open_shift("B2", &id).await), ABANDONED);
    let movement = MoveStock::from_payload(&json!({
        "requestId": id, "shiftId": shift.id, "currency": "USD",
        "direction": "IN", "amount": 5, "reason": "top up",
    }))
    .unwrap();
    assert_eq!(
        refusal(
            db.shifts()
                .move_stock(&branch(MAIN), movement, ACTOR, at(1))
                .await
        ),
        ABANDONED
    );
    let closing = CloseShift::from_payload(&json!({
        "requestId": id, "shiftId": shift.id,
        "expected": { "USD": 10 }, "counted": { "USD": 10 }, "note": "",
    }))
    .unwrap();
    assert_eq!(
        refusal(
            db.shifts()
                .close(&branch(MAIN), closing, ACTOR, at(2))
                .await
        ),
        ABANDONED
    );
    let create = CreateBranch {
        request_id: id.clone(),
        name: "Airport".to_string(),
        from_branch: branch(MAIN),
    };
    assert_eq!(
        refusal(db.branches().create(create, ACTOR, at(3)).await),
        ABANDONED
    );

    assert_eq!(db.count("rates").await, 0);
    assert_eq!(db.count("stock_movements").await, 0);
    assert_eq!(db.count("shifts").await, 1);
    assert_eq!(db.count("branches").await, 2);
    db.finish().await;
}

// ---------------------------------------------------------------------------
// APPLIED
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn an_applied_rate_request_is_applied_and_no_row_is_written() {
    let db = TestDb::start().await;
    let id = request_id();
    db.save_rate(MAIN, &id).await.unwrap();
    assert_eq!(db.abandon(MAIN, &id, 0).await.unwrap(), Applied);
    assert_eq!(db.abandon(MAIN, &id, 1).await.unwrap(), Applied);
    assert_eq!(db.rows().await, vec![]);
    // and the request is still what it was: sent again, it is the same request
    db.save_rate(MAIN, &id).await.unwrap();
    assert_eq!(db.count("rates").await, 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_applied_open_shift_request_is_applied() {
    let db = TestDb::start().await;
    let id = request_id();
    let shift = db.open_shift(MAIN, &id).await.unwrap();
    assert_eq!(db.abandon(MAIN, &id, 0).await.unwrap(), Applied);
    assert_eq!(db.rows().await, vec![]);
    assert_eq!(db.open_shift(MAIN, &id).await.unwrap().id, shift.id);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn applied_stock_movement_and_close_shift_requests_are_applied() {
    let db = TestDb::start().await;
    let shift = db.open_shift(MAIN, &request_id()).await.unwrap();

    let moved = request_id();
    let movement = MoveStock::from_payload(&json!({
        "requestId": moved, "shiftId": shift.id, "currency": "USD",
        "direction": "IN", "amount": 5, "reason": "top up",
    }))
    .unwrap();
    db.shifts()
        .move_stock(&branch(MAIN), movement, ACTOR, at(1))
        .await
        .unwrap();
    assert_eq!(db.abandon(MAIN, &moved, 2).await.unwrap(), Applied);

    let closed = request_id();
    let closing = CloseShift::from_payload(&json!({
        "requestId": closed, "shiftId": shift.id,
        "expected": { "USD": 15 }, "counted": { "USD": 15 }, "note": "",
    }))
    .unwrap();
    db.shifts()
        .close(&branch(MAIN), closing, ACTOR, at(3))
        .await
        .unwrap();
    assert_eq!(db.abandon(MAIN, &closed, 4).await.unwrap(), Applied);

    assert_eq!(db.rows().await, vec![]);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_request_that_created_a_branch_is_applied_in_every_branch() {
    let db = TestDb::start().await;
    let id = request_id();
    let create = CreateBranch {
        request_id: id.clone(),
        name: "Airport".to_string(),
        from_branch: branch(MAIN),
    };
    let airport = db.branches().create(create, ACTOR, at(0)).await.unwrap();
    db.seed_branch("B3").await;

    assert_eq!(db.abandon(MAIN, &id, 1).await.unwrap(), Applied);
    assert_eq!(
        db.abandon(airport.id.as_str(), &id, 2).await.unwrap(),
        Applied
    );
    assert_eq!(db.abandon("B3", &id, 3).await.unwrap(), Applied);
    assert_eq!(db.rows().await, vec![]);
    db.finish().await;
}

// ---------------------------------------------------------------------------
// One branch does not answer for another
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_request_applied_in_another_branch_is_not_applied_here() {
    let db = TestDb::start().await;
    db.seed_branch("B2").await;

    // a rate, an open shift, a movement and a closed shift — all in B2
    let rate = request_id();
    db.save_rate("B2", &rate).await.unwrap();
    let opened = request_id();
    let shift = db.open_shift("B2", &opened).await.unwrap();
    let moved = request_id();
    let movement = MoveStock::from_payload(&json!({
        "requestId": moved, "shiftId": shift.id, "currency": "USD",
        "direction": "IN", "amount": 5, "reason": "top up",
    }))
    .unwrap();
    db.shifts()
        .move_stock(&branch("B2"), movement, ACTOR, at(1))
        .await
        .unwrap();
    let closed = request_id();
    let closing = CloseShift::from_payload(&json!({
        "requestId": closed, "shiftId": shift.id,
        "expected": { "USD": 15 }, "counted": { "USD": 15 }, "note": "",
    }))
    .unwrap();
    db.shifts()
        .close(&branch("B2"), closing, ACTOR, at(2))
        .await
        .unwrap();

    let ids = [&rate, &opened, &moved, &closed];
    for id in ids {
        assert_eq!(db.abandon(MAIN, id, 3).await.unwrap(), Cancelled, "in MAIN");
        assert_eq!(db.abandon("B2", id, 4).await.unwrap(), Applied, "in B2");
    }
    let mut expected: Vec<_> = ids.iter().map(|id| pair(MAIN, id)).collect();
    expected.sort();
    assert_eq!(db.keys().await, expected);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_id_given_up_in_one_branch_is_free_in_another() {
    let db = TestDb::start().await;
    db.seed_branch("B2").await;
    let id = request_id();
    assert_eq!(db.abandon(MAIN, &id, 0).await.unwrap(), Cancelled);

    // B2 knows nothing of it: the id can be applied there …
    db.save_rate("B2", &id).await.unwrap();
    assert_eq!(db.abandon("B2", &id, 1).await.unwrap(), Applied);
    // … and stays given up in MAIN
    assert_eq!(refusal(db.save_rate(MAIN, &id).await), ABANDONED);
    assert_eq!(db.abandon(MAIN, &id, 2).await.unwrap(), Cancelled);
    assert_eq!(db.keys().await, vec![pair(MAIN, &id)]);
    db.finish().await;
}

// ---------------------------------------------------------------------------
// Through BranchTx
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_is_one_write_of_the_branch() {
    let db = TestDb::start().await;
    db.seed_branch("B2").await;
    let tx = db.branch_tx();
    assert_eq!(tx.version(&branch(MAIN)).await.unwrap(), 0);
    assert_eq!(db.abandon(MAIN, &request_id(), 0).await.unwrap(), Cancelled);
    assert_eq!(tx.version(&branch(MAIN)).await.unwrap(), 1);
    assert_eq!(tx.version(&branch("B2")).await.unwrap(), 0);
    assert_eq!(db.abandon("B2", &request_id(), 0).await.unwrap(), Cancelled);
    assert_eq!(tx.version(&branch("B2")).await.unwrap(), 1);
    assert_eq!(tx.version(&branch(MAIN)).await.unwrap(), 1);
    db.finish().await;
}

/// Opens a shift with `id` in `branch_id` inside a `BranchTx::run` that does not
/// commit until the returned sender fires. Returns once the shift is written
/// (and not committed).
async fn open_shift_and_hold(
    db: &TestDb,
    branch_id: &'static str,
    id: &str,
) -> (oneshot::Sender<()>, tokio::task::JoinHandle<AppResult<()>>) {
    let (entered_tx, entered_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();
    let branch_tx = db.branch_tx();
    let shifts = db.shifts();
    let cmd = opening(id);
    let holder = tokio::spawn(async move {
        let b = branch(branch_id);
        let inner = b.clone();
        branch_tx
            .run(&b, move |tx| {
                Box::pin(async move {
                    shifts.open_in(tx, &inner, cmd, ACTOR, at(0)).await?;
                    let _ = entered_tx.send(());
                    let _ = release_rx.await;
                    Ok(())
                })
            })
            .await
    });
    within("the holder writes its shift", entered_rx)
        .await
        .expect("the holder is alive");
    (release_tx, holder)
}

#[tokio::test(flavor = "multi_thread")]
async fn it_waits_for_a_write_of_the_branch_in_flight_and_then_sees_it() {
    let db = TestDb::start().await;
    let id = request_id();
    let (release, holder) = open_shift_and_hold(&db, MAIN, &id).await;

    let use_case = db.abandoning();
    let payload = json!({ "requestId": id });
    let abandoning = tokio::spawn(async move {
        use_case
            .abandon(&branch(MAIN), &payload, ACTOR, at(1))
            .await
    });

    // the open shift is written and not committed: the answer must wait for it
    tokio::time::sleep(STAYS_OUT).await;
    assert!(
        !abandoning.is_finished(),
        "answered while a write of the same branch was in flight"
    );

    release.send(()).expect("the holder is waiting");
    within("the holder commits", holder)
        .await
        .expect("the holder did not panic")
        .expect("the shift is opened");
    let answer = within("the answer after the commit", abandoning)
        .await
        .expect("the use case did not panic");
    assert_eq!(answer.unwrap(), Applied);
    assert_eq!(db.rows().await, vec![]);
    assert_eq!(db.count("shifts").await, 1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_in_flight_in_another_branch_does_not_hold_it_up() {
    let db = TestDb::start().await;
    db.seed_branch("B2").await;
    let id = request_id();
    let (release, holder) = open_shift_and_hold(&db, "B2", &id).await;

    // B2 is busy; MAIN is not
    assert_eq!(db.abandon(MAIN, &id, 1).await.unwrap(), Cancelled);

    release.send(()).expect("the holder is waiting");
    within("the holder commits", holder)
        .await
        .expect("the holder did not panic")
        .expect("the shift is opened");
    assert_eq!(db.abandon("B2", &id, 2).await.unwrap(), Applied);
    assert_eq!(db.keys().await, vec![pair(MAIN, &id)]);
    db.finish().await;
}

/// What a race between giving a request up and applying it may end as.
fn exactly_one_won(
    what: &str,
    abandoned: AppResult<RequestResolution>,
    applied: Result<(), String>,
    rows_abandoned: i64,
    rows_applied: i64,
) -> RequestResolution {
    let resolution = abandoned.unwrap_or_else(|e| panic!("{what}: abandon failed: {e:?}"));
    match (resolution, &applied) {
        (Applied, Ok(())) => {
            assert_eq!((rows_abandoned, rows_applied), (0, 1), "{what}: applied");
        }
        (Cancelled, Err(sentence)) => {
            assert_eq!(sentence, ABANDONED, "{what}");
            assert_eq!((rows_abandoned, rows_applied), (1, 0), "{what}: cancelled");
        }
        other => panic!("{what}: both won or both lost: {other:?}"),
    }
    resolution
}

fn sentence_of<T>(result: AppResult<T>) -> Result<(), String> {
    match result {
        Ok(_) => Ok(()),
        Err(AppError::Domain(sentence)) => Err(sentence),
        Err(other) => panic!("not a refusal: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandoning_and_saving_a_rate_at_once_exactly_one_wins() {
    let db = TestDb::start().await;
    for round in 0..24 {
        let branch_id = format!("R{round}");
        db.seed_branch(&branch_id).await;
        let id = request_id();
        let start = Arc::new(Barrier::new(2));

        let abandoning = {
            let (use_case, start) = (db.abandoning(), Arc::clone(&start));
            let (b, payload) = (branch(&branch_id), json!({ "requestId": id }));
            tokio::spawn(async move {
                start.wait().await;
                use_case.abandon(&b, &payload, ACTOR, at(1)).await
            })
        };
        let saving = {
            let (rates, start) = (db.rates(), Arc::clone(&start));
            let (b, cmd) = (branch(&branch_id), usd_rate(&id));
            tokio::spawn(async move {
                start.wait().await;
                rates.save_rate(&b, cmd, ACTOR, at(1)).await
            })
        };
        let abandoned = within("abandon", abandoning).await.expect("no panic");
        let saved = within("save", saving).await.expect("no panic");

        let rows_abandoned: i64 =
            sqlx::query_scalar("SELECT count(*) FROM abandoned_requests WHERE branch_id = $1")
                .bind(&branch_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let rows_rates: i64 = sqlx::query_scalar("SELECT count(*) FROM rates WHERE branch_id = $1")
            .bind(&branch_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        exactly_one_won(
            &format!("round {round}"),
            abandoned,
            sentence_of(saved),
            rows_abandoned,
            rows_rates,
        );
    }
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandoning_and_opening_a_shift_at_once_exactly_one_wins() {
    let db = TestDb::start().await;
    for round in 0..24 {
        let branch_id = format!("S{round}");
        db.seed_branch(&branch_id).await;
        let id = request_id();
        let start = Arc::new(Barrier::new(2));

        let abandoning = {
            let (use_case, start) = (db.abandoning(), Arc::clone(&start));
            let (b, payload) = (branch(&branch_id), json!({ "requestId": id }));
            tokio::spawn(async move {
                start.wait().await;
                use_case.abandon(&b, &payload, ACTOR, at(1)).await
            })
        };
        let opening_it = {
            let (shifts, start) = (db.shifts(), Arc::clone(&start));
            let (b, cmd) = (branch(&branch_id), opening(&id));
            tokio::spawn(async move {
                start.wait().await;
                shifts.open(&b, cmd, ACTOR, at(1)).await
            })
        };
        let abandoned = within("abandon", abandoning).await.expect("no panic");
        let opened = within("open", opening_it).await.expect("no panic");

        let rows_abandoned: i64 =
            sqlx::query_scalar("SELECT count(*) FROM abandoned_requests WHERE branch_id = $1")
                .bind(&branch_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let rows_shifts: i64 =
            sqlx::query_scalar("SELECT count(*) FROM shifts WHERE branch_id = $1")
                .bind(&branch_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        exactly_one_won(
            &format!("round {round}"),
            abandoned,
            sentence_of(opened),
            rows_abandoned,
            rows_shifts,
        );
    }
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_abandoning_the_same_id_at_once_leave_one_row() {
    let db = TestDb::start().await;
    let id = request_id();
    let start = Arc::new(Barrier::new(8));
    let mut tasks = Vec::new();
    for n in 0..8 {
        let (use_case, start) = (db.abandoning(), Arc::clone(&start));
        let payload = json!({ "requestId": id });
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            use_case
                .abandon(&branch(MAIN), &payload, ACTOR, at(n))
                .await
        }));
    }
    for task in tasks {
        let answer = within("abandon", task).await.expect("no panic");
        assert_eq!(answer.unwrap(), Cancelled);
    }
    assert_eq!(db.keys().await, vec![pair(MAIN, &id)]);
    assert_eq!(db.branch_tx().version(&branch(MAIN)).await.unwrap(), 8);
    db.finish().await;
}

/// CA-1 (#617): a shift's opener must name a user (`shifts.teller_id`, design #582 §8 M1 item 3). The user an
/// opener id resolves to: `users.legacy_employee_id = id`, or, for `"owner"`, the organisation-wide EXECUTIVE +
/// ADMIN user. Seeded once per id; no test's expectation depends on it. Added by the ticket author in CA-1's
/// up-front commit.
async fn seed_opener(pool: &PgPool, id: &str) {
    if id == "owner" {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users u WHERE u.legacy_employee_id IS NULL AND EXISTS \
             (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL))",
        )
        .fetch_one(pool)
        .await
        .expect("look for the owner's user");
        if !exists {
            sqlx::query(
                "WITH o AS (SELECT id FROM organizations WHERE code = 'XC'), \
                 u AS (INSERT INTO users (organization_id, name, status, created_by) \
                       SELECT id, 'owner', 'PENDING', 'test' FROM o RETURNING id, organization_id) \
                 INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                 SELECT u.id, r.code, u.organization_id, NULL FROM u, (VALUES ('EXECUTIVE'), ('ADMIN')) AS r(code)",
            )
            .execute(pool)
            .await
            .expect("seed the owner's user");
        }
        return;
    }
    sqlx::query(
        "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
         SELECT id, $1, 'PENDING', 'test', $1 FROM organizations WHERE code = 'XC' \
         ON CONFLICT (legacy_employee_id) DO NOTHING",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed an opener's user");
}
