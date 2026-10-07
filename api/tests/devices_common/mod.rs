//! Written before the implementation (ticket T13). Do not edit, delete or weaken.
//!
//! Shared by the device tests: a database per test, the people the service
//! asks (which exchanges exist, which id comes next), the v2 recording, and the
//! helpers that read what is really stored.
//!
//! The single `pub use` block below is every path of the implementation the
//! tests name.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an
//! existing database whose name ends in `_test`, and its role must be allowed
//! to create databases. When the variable is absent every test FAILS; none
//! skips. Each test gets a database of its own through
//! `routes_common::ScratchDatabase`, migrated by `bootstrap::connect` — the
//! same way the server migrates.
#![allow(dead_code, unused_imports)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use futures_free::BoxFuture;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

pub use xchange::infrastructure::devices::device_commands::{QueueJob, SavedExchange};
pub use xchange::infrastructure::devices::device_event::DeviceEvent;
pub use xchange::infrastructure::devices::device_job::{DeviceJob, JobKind, JobStatus};
pub use xchange::infrastructure::devices::device_service::{DeviceOverview, DeviceService};
pub use xchange::infrastructure::devices::job_ids::{JobIds, UuidJobIds};
pub use xchange::infrastructure::devices::saved_exchanges::{ExchangeState, SavedExchanges};

use crate::routes_common::ScratchDatabase;

/// The boxed future a port answers with, spelled out so the tests need no
/// crate for it.
mod futures_free {
    use std::future::Future;
    use std::pin::Pin;
    pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
}

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(30);

pub const MAIN: &str = "MAIN";
/// A second branch every test database has.
pub const B9: &str = "B9";
pub const ACTOR: &str = "front1";

// The sentences v2 answers with in this context (fixture key `sentences`).
pub const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const BAD_KIND: &str = "ประเภทอุปกรณ์ไม่ถูกต้อง";
pub const BAD_REQUEST_ID: &str = "รหัสคำขอไม่ถูกต้อง";
pub const PRINT_NEEDS_REF: &str = "เลือกรายการก่อนพิมพ์";
pub const NOT_COMPLETED_HERE: &str = "ไม่พบรายการสำเร็จในสาขานี้";
pub const REASON_TOO_LONG: &str = "เหตุผลยาวเกินกำหนด";
pub const DRAWER_NEEDS_REASON: &str = "ระบุเหตุผลเปิดลิ้นชัก";
pub const REQUEST_CHANGED: &str = "คำขอเดิมมีข้อมูลเปลี่ยน";
pub const NO_SUCH_JOB: &str = "ไม่พบงาน";
pub const RETRY_ONLY_FAILED: &str = "ลองใหม่ได้เฉพาะงานที่ล้มเหลว";
pub const NOT_HERE: &str = "ไม่พบรายการในสาขานี้";
pub const BAD_STATE: &str = "สถานะไม่ถูกต้อง";
pub const COUNT_FIRST: &str = "ยืนยันการนับเงินก่อนบันทึกรายการเงินสด";

/// A completed exchange of MAIN in every test database's fake.
pub const REF1: &str = "XC-20261006-000001";
pub const REF2: &str = "XC-20261006-000002";
/// A completed exchange of B9.
pub const REF9: &str = "XC-20261006-000009";

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

/// An instant as the tests read it back from the database: UTC, six decimals.
pub fn micros(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// The instant `t` cut to whole milliseconds, written with six decimals.
pub fn cut(t: DateTime<Utc>) -> String {
    micros(
        Utc.timestamp_millis_opt(t.timestamp_millis())
            .single()
            .expect("a valid instant"),
    )
}

/// `2026-10-06T05:00:01.123Z`, as the recording writes time.
pub fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text}: {e}"))
        .with_timezone(&Utc)
}

/// The sentence of a refusal. Panics on success and on any error that is not
/// `AppError::Domain` — a refusal reaches the screen as its sentence and nothing else.
pub fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

/// Two JSON values with numbers compared as numbers (`8125` is `8125.0`).
pub fn same_json(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_json(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, p)| y.get(k).is_some_and(|q| same_json(p, q)))
        }
        _ => a == b,
    }
}

// ───────────────────────────── the two things the service asks ──────────────

/// Hands out the ids it was given, in order. When it has none left it hands
/// out a text no test expects, so a service that draws more than v2 did is
/// seen in what it stores, not as a panic inside it.
#[derive(Default)]
pub struct FakeIds {
    next: Mutex<VecDeque<String>>,
    extra: AtomicU64,
}

impl FakeIds {
    pub fn give(&self, ids: &[String]) {
        let mut next = self.next.lock().expect("ids");
        next.clear();
        next.extend(ids.iter().cloned());
    }

    /// How many ids were asked for when none was left to hand out.
    pub fn asked_for_too_many(&self) -> u64 {
        self.extra.load(Ordering::SeqCst)
    }

    pub fn give_one(&self, id: &str) {
        self.give(&[id.to_owned()]);
    }
}

impl JobIds for FakeIds {
    fn next(&self) -> String {
        if let Some(id) = self.next.lock().expect("ids").pop_front() {
            return id;
        }
        format!(
            "not-expected-{:024}",
            self.extra.fetch_add(1, Ordering::SeqCst)
        )
    }
}

/// A job id in v2's shape, different for every `n`.
pub fn job_id(n: u32) -> String {
    format!("0b0b0b0b-de71-4ce5-8000-{n:012x}")
}

/// A request id in v2's shape, different for every `n`.
pub fn request_id(n: u32) -> String {
    format!("cccccccc-0000-4000-8000-{n:012x}")
}

/// The exchanges the service is told exist: (branch, reference) → state.
#[derive(Default)]
pub struct FakeExchanges {
    known: Mutex<HashMap<(String, String), ExchangeState>>,
}

impl FakeExchanges {
    pub fn set(&self, branch: &str, reference: &str, state: ExchangeState) {
        self.known
            .lock()
            .expect("exchanges")
            .insert((branch.to_owned(), reference.to_owned()), state);
    }

    pub fn clear(&self) {
        self.known.lock().expect("exchanges").clear();
    }
}

impl SavedExchanges for FakeExchanges {
    fn find<'a>(
        &'a self,
        branch: &'a BranchId,
        reference: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<ExchangeState>>> {
        let found = self
            .known
            .lock()
            .expect("exchanges")
            .get(&(branch.as_str().to_owned(), reference.to_owned()))
            .copied();
        Box::pin(async move { Ok(found) })
    }
}

// ───────────────────────────── the database of one test ─────────────────────

pub struct Devices {
    pub pool: PgPool,
    pub service: DeviceService,
    pub ids: Arc<FakeIds>,
    pub exchanges: Arc<FakeExchanges>,
    database: ScratchDatabase,
}

impl Devices {
    /// A migrated database with the branches MAIN (from the migration) and B9,
    /// the three exchanges `REF1`, `REF2` (MAIN) and `REF9` (B9) completed.
    pub async fn start() -> Self {
        let database = ScratchDatabase::create("t1b_dev").await;
        let pool = xchange::bootstrap::connect(&database.settings())
            .await
            .expect("connect and migrate the scratch database");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'Nine')")
            .bind(B9)
            .execute(&pool)
            .await
            .expect("insert the second branch");
        let ids = Arc::new(FakeIds::default());
        let exchanges = Arc::new(FakeExchanges::default());
        exchanges.set(MAIN, REF1, ExchangeState::Completed);
        exchanges.set(MAIN, REF2, ExchangeState::Completed);
        exchanges.set(B9, REF9, ExchangeState::Completed);
        let service = DeviceService::new(
            BranchTx::new(Transactor::new(pool.clone())),
            exchanges.clone(),
            ids.clone(),
        );
        Self {
            pool,
            service,
            ids,
            exchanges,
            database,
        }
    }

    /// The same service on a pool of exactly one connection: a command that
    /// reads or writes beside its own transaction finds no connection to do it on.
    pub async fn on_one_connection(&self) -> (DeviceService, PgPool) {
        let pool = self.database.inspect().await;
        let service = DeviceService::new(
            BranchTx::new(Transactor::new(pool.clone())),
            self.exchanges.clone(),
            self.ids.clone(),
        );
        (service, pool)
    }

    pub fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    /// Reached only by a test that passed; a failed test leaves its database
    /// behind (the next run sweeps it).
    pub async fn finish(self) {
        self.pool.close().await;
        self.database.drop().await;
    }

    /// Queue a print job for `REF1` in MAIN with a known id; returns that id.
    pub async fn queued_print(&self, n: u32) -> String {
        self.queued(MAIN, "PRINT_RECEIPT", REF1, "", n).await
    }

    pub async fn queued(
        &self,
        branch_id: &str,
        kind: &str,
        reference: &str,
        reason: &str,
        n: u32,
    ) -> String {
        let id = job_id(n);
        self.ids.give_one(&id);
        let job = self
            .service
            .queue(
                &branch(branch_id),
                QueueJob {
                    kind: kind.to_owned(),
                    reference: reference.to_owned(),
                    request_id: request_id(n),
                    reason: reason.to_owned(),
                },
                ACTOR,
                at(0),
            )
            .await
            .expect("queue a job");
        assert_eq!(job.id, id, "the job takes the id the source handed out");
        id
    }

    /// Switch a branch's printer behind the service's back.
    pub async fn printer(&self, branch_id: &str, online: bool) {
        sqlx::query(
            "INSERT INTO device_settings (branch_id, key, value) VALUES ($1, 'printerOnline', $2) \
             ON CONFLICT (branch_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(branch_id)
        .bind(if online { "1" } else { "0" })
        .execute(&self.pool)
        .await
        .expect("set the printer switch");
    }

    /// Set a job's status behind the service's back.
    pub async fn force_status(&self, id: &str, status: &str) {
        let done = sqlx::query("UPDATE device_jobs SET status = $1 WHERE id = $2")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("update the job");
        assert_eq!(done.rows_affected(), 1, "job {id}");
    }

    /// Make the next `BranchTx::run` on `branch_id` fail at its last statement:
    /// the version cannot be incremented past the largest bigint. Everything the
    /// command wrote on the transaction is rolled back; anything it wrote
    /// elsewhere stays.
    pub async fn break_the_commit(&self, branch_id: &str) {
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version) VALUES ($1, $2) \
             ON CONFLICT (branch_id) DO UPDATE SET version = excluded.version",
        )
        .bind(branch_id)
        .bind(i64::MAX)
        .execute(&self.pool)
        .await
        .expect("set the branch version");
    }

    pub async fn version(&self, branch_id: &str) -> i64 {
        self.branch_tx()
            .version(&branch(branch_id))
            .await
            .expect("read the branch version")
    }

    /// Everything stored, in the recording's words.
    pub async fn stored(&self) -> Stored {
        Stored {
            jobs: self.jobs().await,
            events: self.events().await,
            settings: self.settings().await,
        }
    }

    /// Every job by id, as `{id, request_id, branch_id, ref, kind, status, actor, created, updated}`
    /// with the two instants in six decimals.
    pub async fn jobs(&self) -> BTreeMap<String, Value> {
        type Row = (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, request_id, branch_id, ref, kind, status, actor, \
             to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
             to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
             FROM device_jobs",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read device_jobs");
        rows.into_iter()
            .map(|r| {
                (
                    r.0.clone(),
                    json!({
                        "id": r.0, "request_id": r.1, "branch_id": r.2, "ref": r.3, "kind": r.4,
                        "status": r.5, "actor": r.6, "created": r.7, "updated": r.8,
                    }),
                )
            })
            .collect()
    }

    /// Every event in the order written, as `{dedupe, branch_id, actor, kind, ref, detail, at}`.
    pub async fn events(&self) -> Vec<Value> {
        type Row = (
            Option<String>,
            String,
            String,
            String,
            String,
            String,
            String,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT dedupe, branch_id, actor, kind, ref, detail::text, \
             to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
             FROM device_events ORDER BY seq",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read device_events");
        rows.into_iter()
            .map(|r| {
                let detail: Value = serde_json::from_str(&r.5).expect("detail is JSON");
                json!({
                    "dedupe": r.0, "branch_id": r.1, "actor": r.2, "kind": r.3, "ref": r.4,
                    "detail": detail, "at": r.6,
                })
            })
            .collect()
    }

    /// Every setting as (branch, key) → value.
    pub async fn settings(&self) -> BTreeMap<(String, String), String> {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT branch_id, key, value FROM device_settings")
                .fetch_all(&self.pool)
                .await
                .expect("read device_settings");
        rows.into_iter().map(|r| ((r.0, r.1), r.2)).collect()
    }
}

/// The three tables, read whole.
#[derive(Debug, Clone, Default)]
pub struct Stored {
    pub jobs: BTreeMap<String, Value>,
    pub events: Vec<Value>,
    pub settings: BTreeMap<(String, String), String>,
}

impl Stored {
    pub fn same_as(&self, other: &Stored) -> bool {
        self.settings == other.settings
            && self.jobs.len() == other.jobs.len()
            && self
                .jobs
                .iter()
                .all(|(id, job)| other.jobs.get(id).is_some_and(|o| same_json(job, o)))
            && self.events.len() == other.events.len()
            && self
                .events
                .iter()
                .zip(&other.events)
                .all(|(a, b)| same_json(a, b))
    }
}

/// A job the service returned, in the recording's words.
pub fn job_json(job: &DeviceJob) -> Value {
    json!({
        "id": job.id, "request_id": job.request_id, "branch_id": job.branch_id,
        "ref": job.reference, "kind": job.kind.as_str(), "status": job.status.as_str(),
        "actor": job.actor, "created": micros(job.created_at), "updated": micros(job.updated_at),
    })
}

// ───────────────────────────── the branch lock, seen from outside ───────────

/// Takes the branch lock from a transaction of the test's own, the way
/// `BranchTx::run` takes it.
pub async fn lock_branch(
    pool: &PgPool,
    branch_id: &str,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.expect("begin the locking transaction");
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) \
         ON CONFLICT (branch_id) DO NOTHING",
    )
    .bind(branch_id)
    .execute(&mut *tx)
    .await
    .expect("create the branch's version row");
    sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
        .bind(branch_id)
        .execute(&mut *tx)
        .await
        .expect("lock the branch's version row");
    tx
}

/// How many backends of this test's database wait for a lock right now.
pub async fn waiting_backends(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(DISTINCT l.pid) FROM pg_locks l \
         JOIN pg_stat_activity a ON a.pid = l.pid \
         WHERE a.datname = current_database() AND NOT l.granted",
    )
    .fetch_one(pool)
    .await
    .expect("read pg_locks")
}

/// Returns once a backend of this test's database waits for a lock. Asks
/// PostgreSQL again and again; never sleeps.
pub async fn until_somebody_waits(pool: &PgPool) {
    within(MUST_FINISH, "a backend waits for the branch lock", async {
        while waiting_backends(pool).await == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
}

// ───────────────────────────── the v2 recording ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Recording {
    pub counts: Counts,
    pub sentences: BTreeMap<String, String>,
    #[serde(rename = "countFirst")]
    pub count_first: String,
    #[serde(rename = "countConfirmed")]
    pub count_confirmed: Vec<CountCase>,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
pub struct Counts {
    pub scenarios: usize,
    pub steps: usize,
    pub refused: usize,
    pub stored: usize,
    #[serde(rename = "v3Differs")]
    pub v3_differs: usize,
}

#[derive(Debug, Deserialize)]
pub struct CountCase {
    pub cash: bool,
    pub confirmed: bool,
    pub refused: bool,
}

#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// branch → the exchanges v2 was told the branch has: `{id, status}`.
    pub exchanges: BTreeMap<String, Vec<Exchange>>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
pub struct Exchange {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct Step {
    pub call: String,
    pub actor: String,
    pub branch: String,
    pub now: String,
    pub args: Value,
    #[serde(default)]
    pub note: Option<String>,
    /// `differs` on the one step where v3 is meant to answer differently.
    #[serde(default)]
    pub v3: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub result: Option<Value>,
    pub ids: Vec<String>,
    pub stored: StoredDelta,
    pub changed: bool,
}

#[derive(Debug, Deserialize)]
pub struct StoredDelta {
    pub events: Vec<Value>,
    pub jobs: Vec<Value>,
    pub settings: Vec<Value>,
}

pub fn recording() -> Recording {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/devices/v2_device_cases.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// `…05:00:01.123Z` → `…05:00:01.123000Z`: the recording's instants in the six
/// decimals the tests read from the database.
pub fn six_decimals(iso: &str) -> String {
    micros(instant(iso))
}

/// A recorded job with its instants in six decimals.
pub fn recorded_job(job: &Value) -> Value {
    let mut job = job.clone();
    for key in ["created", "updated"] {
        let text = job[key].as_str().expect("an instant").to_owned();
        job[key] = Value::String(six_decimals(&text));
    }
    job
}

/// A recorded event as v3 stores it: the instant in six decimals, and in the
/// count line the baht figure as the exact product (v2 multiplied two doubles;
/// the recording carries both, `thb` and `thbExact`).
pub fn recorded_event(event: &Value) -> Value {
    let mut event = event.clone();
    let at = event["at"].as_str().expect("an instant").to_owned();
    event["at"] = Value::String(six_decimals(&at));
    if let Some(exact) = event["detail"].get("thbExact").cloned() {
        let exact: f64 = exact
            .as_str()
            .expect("thbExact is text")
            .parse()
            .expect("thbExact is a number");
        let detail = event["detail"].as_object_mut().expect("detail");
        detail.remove("thbExact");
        detail.insert("thb".to_owned(), json!(exact));
    }
    event
}
