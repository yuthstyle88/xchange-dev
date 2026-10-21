//! Ticket #597 (CM-1), amended by the lane controller's rulings on #691 / #597
//! (option A, then X): `run` takes the transaction the endpoint opens, and
//! `finish_in` writes the answer in that same transaction, so a command's writes
//! and its stored answer commit together or not at all. A failed `run` (or a
//! failed `finish_in`) leaves none of the command's writes and releases the
//! claim, so a retry with the same key and the same body runs again and succeeds
//! once; a failure is never replayed; and a `run` that outlives the lease and is
//! taken over keeps none of its writes. Written before the implementation by the ticket
//! author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! Design: `docs/design/blueprint/rates.md` §5.6 (how a claim ends) and §6
//! (`CommandAction::run(&self, tx, branch, who, input, key)`, R-TX).
//!
//! Every test command writes one row into `test_effects` (a table this file
//! creates in its schema) through `run`'s transaction, so "the command's writes
//! happened" is measured, not assumed.
//! Store-level rules (token, lease) are in `idempotency_lease_test.rs`.
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. They fail without it; they do not skip.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Notify;
use ts_rs::TS;
use xc_auth::domain::access::Actor as AccessActor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::idempotency::store::IdempotencyKey;
use xchange::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE_D1: &str = "/api/test-flaky/d-1";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The v3 command seam tests need a real PostgreSQL and do not skip. Point it \
                 at an existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`."
        );
        let schema = unique("v3release");
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
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B02', 'B02')")
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        sqlx::query("CREATE TABLE test_effects (key text NOT NULL, run int NOT NULL)")
            .execute(&pool)
            .await
            .expect("create test_effects");
        Self { pool, schema }
    }

    /// `(operation, key)` of every `idempotency_records` row, in that order.
    async fn records(&self) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT operation, key FROM idempotency_records ORDER BY operation, key",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read idempotency_records (migration 3 creates it)")
    }

    /// Drop the schema. Reached only by a test that passed.
    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ measuring

impl TestDb {
    /// The `run` numbers whose writes are committed, in order.
    async fn effects(&self) -> Vec<i32> {
        sqlx::query_scalar("SELECT run FROM test_effects ORDER BY run")
            .fetch_all(&self.pool)
            .await
            .expect("read test_effects")
    }

    /// Make the record of `key` look claimed 61 s ago (past the 60 s lease).
    async fn expire(&self, key: &str) {
        let n = sqlx::query(
            "UPDATE idempotency_records SET created_at = now() - interval '61 seconds' \
             WHERE operation = 'test-flaky' AND key = $1",
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .expect("expire the record")
        .rows_affected();
        assert_eq!(n, 1, "one record of {key} to expire");
    }

    /// Wait until the record of `key` has been claimed afresh (taken over).
    async fn wait_for_takeover(&self, key: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let fresh: Option<bool> = sqlx::query_scalar(
                "SELECT created_at > now() - interval '30 seconds' FROM idempotency_records \
                 WHERE operation = 'test-flaky' AND key = $1",
            )
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .expect("read the record");
            if fresh == Some(true) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the second claim never took {key} over"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

// ------------------------------------------------------------ the command

/// The owner is signed in on every request.
struct OwnerCallers;

impl SessionReader for OwnerCallers {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move { test_callers::callers(Some(test_callers::owner("owner-1")), None) })
    }
}

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct FlakyInput {
    id: String,
    amount: String,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct FlakyDone {
    id: String,
    amount: String,
    run: usize,
}

/// What the n-th run does after writing its `test_effects` row.
#[derive(Clone, Copy)]
enum Step {
    /// Answers `Ok`.
    Succeed,
    /// Answers `Err` with this refusal.
    Fail(fn() -> V3Error),
    /// Deletes its own idempotency record in its transaction and answers `Ok`,
    /// so the endpoint's `finish_in` finds nothing to finish: "run committed,
    /// then finish failed" made reproducible.
    SpoilItsOwnFinish,
    /// Signals `started`, waits for `go`, then answers `Ok`: a run that
    /// outlives the lease.
    WaitForGo,
}

struct Flaky {
    runs: Arc<AtomicUsize>,
    plan: Vec<Step>,
    started: Arc<Notify>,
    go: Arc<Notify>,
}

impl CommandAction for Flaky {
    const NAME: &'static str = "test-flaky";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Input = FlakyInput;
    type Output = FlakyDone;
    const ROUTE: &'static str = "/api/test-flaky/{id}";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &AccessActor,
        input: FlakyInput,
        key: IdempotencyKey,
    ) -> Result<FlakyDone, V3Error> {
        let run = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
        sqlx::query("INSERT INTO test_effects (key, run) VALUES ($1, $2)")
            .bind(key.as_str())
            .bind(run as i32)
            .execute(&mut **tx)
            .await
            .expect("write the effect in run's transaction");
        match self.plan.get(run - 1).copied().unwrap_or(Step::Succeed) {
            Step::Succeed => {}
            Step::Fail(error) => return Err(error()),
            Step::SpoilItsOwnFinish => {
                sqlx::query("DELETE FROM idempotency_records WHERE operation = $1 AND key = $2")
                    .bind(Self::NAME)
                    .bind(key.as_str())
                    .execute(&mut **tx)
                    .await
                    .expect("delete the own record in run's transaction");
            }
            Step::WaitForGo => {
                self.started.notify_one();
                self.go.notified().await;
            }
        }
        Ok(FlakyDone {
            id: input.id,
            amount: input.amount,
            run,
        })
    }
}

fn stale_version() -> V3Error {
    V3Error::new(
        "staleVersion",
        None,
        "the object moved",
        StatusCode::CONFLICT,
    )
}

fn server_error() -> V3Error {
    V3Error::new(
        "serverError",
        None,
        "a transient failure",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
}

struct Answer {
    status: StatusCode,
    body: Value,
    text: String,
}

async fn post(db: &TestDb, flaky: Arc<Flaky>, amount: &str, key: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(OwnerCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                CommandEndpoint::register(cfg, flaky);
                bootstrap::configure(cfg, state);
            })
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(ROUTE_D1)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", key))
        .set_payload(json!({ "amount": amount }).to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, body, text }
}

fn flaky(plan: Vec<Step>) -> (Arc<Flaky>, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(Flaky {
            runs: runs.clone(),
            plan,
            started: Arc::new(Notify::new()),
            go: Arc::new(Notify::new()),
        }),
        runs,
    )
}

fn is_done(a: &Answer, amount: &str, run: usize, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.text);
    assert_eq!(
        a.body,
        json!({ "id": "d-1", "amount": amount, "run": run }),
        "{what}"
    );
}

// ------------------------------------------------------------ a failed run

async fn a_failed_run_is_retried_with_the_same_key(
    error: fn() -> V3Error,
    status: u16,
    code: &str,
) {
    let db = TestDb::start().await;
    let (command, runs) = flaky(vec![Step::Fail(error)]);

    let first = post(&db, command.clone(), "100.00", "k-retry").await;
    assert_eq!(
        first.status.as_u16(),
        status,
        "the failure answers as run said; body {}",
        first.text
    );
    assert_eq!(
        first.body["error"]["code"],
        json!(code),
        "body {}",
        first.text
    );
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(
        db.effects().await.is_empty(),
        "a failed run keeps none of its writes"
    );
    assert!(
        db.records().await.is_empty(),
        "a failed run leaves no idempotency record: {:?}",
        db.records().await
    );

    let second = post(&db, command.clone(), "100.00", "k-retry").await;
    is_done(&second, "100.00", 2, "the same key and body run again");
    assert_eq!(runs.load(Ordering::SeqCst), 2, "run ran a second time");

    let third = post(&db, command.clone(), "100.00", "k-retry").await;
    is_done(&third, "100.00", 2, "the success is replayed");
    assert_eq!(
        runs.load(Ordering::SeqCst),
        2,
        "and run is not called again"
    );
    assert_eq!(
        db.effects().await,
        vec![2],
        "exactly one run's writes are kept"
    );
    assert_eq!(
        db.records().await,
        vec![("test-flaky".to_string(), "k-retry".to_string())],
        "one record, the success's"
    );
    db.finish().await;
}

/// A refusal from `run` (a 4xx) releases the claim: the retry runs and succeeds once.
#[actix_web::test]
async fn a_refused_run_releases_its_key_and_the_same_key_runs_again_once() {
    a_failed_run_is_retried_with_the_same_key(stale_version, 409, "staleVersion").await;
}

/// A transient server error from `run` (a 5xx) releases the claim too: the
/// retry that idempotency exists for is not refused `idempotencyConflict`.
#[actix_web::test]
async fn a_server_error_in_run_releases_its_key_and_the_same_key_runs_again_once() {
    a_failed_run_is_retried_with_the_same_key(server_error, 500, "serverError").await;
}

/// The failure is never stored: after two failures the third call with the
/// same key still runs, and a different body under the released key is not a
/// conflict either (there is nothing left to conflict with).
#[actix_web::test]
async fn a_failure_is_never_replayed_and_leaves_nothing_to_conflict_with() {
    let db = TestDb::start().await;
    let (command, runs) = flaky(vec![Step::Fail(stale_version), Step::Fail(stale_version)]);

    for n in 1..=2 {
        let a = post(&db, command.clone(), "100.00", "k-twice").await;
        assert_eq!(
            a.status,
            StatusCode::CONFLICT,
            "failure {n}; body {}",
            a.text
        );
        assert_eq!(
            a.body["error"]["code"],
            json!("staleVersion"),
            "failure {n} is run's own answer"
        );
    }
    let a = post(&db, command.clone(), "250.00", "k-twice").await;
    is_done(&a, "250.00", 3, "another body under the released key runs");
    assert_eq!(runs.load(Ordering::SeqCst), 3);
    assert_eq!(db.effects().await, vec![3]);
    db.finish().await;
}

// ------------------------------------------------------------ ruling X

/// 8ba9121f's case 5a, "run committed, then finish failed": with `finish_in` in
/// `run`'s transaction it cannot leave a committed command behind. A failed
/// `finish_in` rolls back the command's writes and releases the claim, so the
/// retry runs once and exactly one run's writes exist.
#[actix_web::test]
async fn a_failed_finish_rolls_back_the_run_so_the_retry_leaves_exactly_one_effect() {
    let db = TestDb::start().await;
    let (command, runs) = flaky(vec![Step::SpoilItsOwnFinish]);

    let first = post(&db, command.clone(), "100.00", "k-finish").await;
    assert!(
        !first.status.is_success(),
        "a run whose finish failed is not answered as a success; got {} {}",
        first.status,
        first.text
    );
    assert!(
        db.effects().await.is_empty(),
        "the run's writes rolled back with the failed finish_in: {:?}",
        db.effects().await
    );
    assert!(
        db.records().await.is_empty(),
        "and its claim was released: {:?}",
        db.records().await
    );

    let second = post(&db, command.clone(), "100.00", "k-finish").await;
    is_done(&second, "100.00", 2, "the retry runs");
    let third = post(&db, command.clone(), "100.00", "k-finish").await;
    is_done(&third, "100.00", 2, "then replays");
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert_eq!(
        db.effects().await,
        vec![2],
        "exactly one run's writes exist"
    );
    db.finish().await;
}

/// 8ba9121f's case 5b, a `run` that outlives the lease: the retry takes the
/// claim over while the first run is still working. The first run's
/// `finish_in` then finds its token taken over and fails, so its writes roll
/// back; only the taker's writes and answer remain.
#[actix_web::test]
async fn a_run_that_outlives_the_lease_and_is_taken_over_keeps_none_of_its_writes() {
    let db = TestDb::start().await;
    let (command, runs) = flaky(vec![Step::WaitForGo, Step::Succeed]);
    let started = command.started.clone();
    let go = command.go.clone();

    let slow = post(&db, command.clone(), "100.00", "k-slow");
    let retry = async {
        started.notified().await;
        db.expire("k-slow").await;
        let taker = post(&db, command.clone(), "100.00", "k-slow");
        let release = async {
            db.wait_for_takeover("k-slow").await;
            go.notify_one();
        };
        let (answer, ()) = tokio::join!(taker, release);
        answer
    };
    let (first, second) = tokio::join!(slow, retry);

    assert!(
        !first.status.is_success(),
        "the taken-over run is not answered as a success; got {} {}",
        first.status,
        first.text
    );
    is_done(&second, "100.00", 2, "the taker's run");
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert_eq!(
        db.effects().await,
        vec![2],
        "only the taker's writes are kept; the slow run's rolled back"
    );
    let third = post(&db, command.clone(), "100.00", "k-slow").await;
    is_done(
        &third,
        "100.00",
        2,
        "the taker's answer is the one replayed",
    );
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    db.finish().await;
}
