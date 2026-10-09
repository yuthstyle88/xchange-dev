//! Ticket #704 Part 1 (CM-2) — a command that writes two branches declares `Locks::Pair`, and
//! `CommandEndpoint` opens `BranchTx::run_pair` for it: both branch locks in the one total order
//! (`MAIN` first, then ascending `branch_id`), both branch versions moved. Written before the
//! implementation by the ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §6 "Locks" and R-TX (#701, PR 703); ledger L-5
//! (`docs/design/blueprint/ledger-inventory.md`); LG-0 `BranchTx::run_pair` (`xc-kernel/src/branch_tx.rs`).
//!
//! Signatures this test assumes:
//! - `xchange::presentation::http::v3::command::Locks`, `pub enum Locks { One(BranchId), Pair(BranchId, BranchId) }`.
//! - `CommandAction::locks(&self, branch: &BranchId, input: &Self::Input) -> Locks`, a provided method whose
//!   default is `Locks::One(branch.clone())`.
//! - `CommandEndpoint` calls `locks()` after reading `Input` and before `claim`. A `Pair` that does not contain
//!   `branch` (the caller's checked branch) is refused `403` with code `outOfScope`, before any idempotency
//!   record or write. `One(b)` ⇒ `BranchTx::run(b)`; `Pair(a, b)` ⇒ `BranchTx::run_pair(a, b)`.
//!
//! Every test command writes one `test_marks` row per branch of its pair, through `run`'s `tx`, so what was
//! written is measured, not assumed. Branch versions are read from `branch_versions`.
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing database whose name ends in
//! `_test`. They fail without it; they do not skip.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use ts_rs::TS;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::idempotency::store::IdempotencyKey;
use xchange::presentation::http::v3::command::{CommandAction, CommandEndpoint, Locks};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUNDS: usize = 20;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
        let schema = unique("v3locks");
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
        sqlx::query(
            // `MAIN` is seeded by the migrations.
            "INSERT INTO branches (id, name) VALUES ('A-1', 'A1'), ('XC-B02', 'B02'), ('XC-B03', 'B03')",
        )
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        sqlx::query("CREATE TABLE test_marks (branch_id text NOT NULL, tag text NOT NULL)")
            .execute(&pool)
            .await
            .expect("create test_marks");
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
    /// The branch's committed version; 0 before its first run.
    async fn version(&self, branch: &str) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT version FROM branch_versions WHERE branch_id = $1")
            .bind(branch)
            .fetch_optional(&self.pool)
            .await
            .expect("read branch_versions")
            .unwrap_or(0)
    }

    async fn marks(&self, branch: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM test_marks WHERE branch_id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .expect("count test_marks")
    }
}

// ------------------------------------------------------------ callers

/// `x-test-as: staff:<ROLE>:<branch>`: one staff session at that branch.
struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let mut parts = spec.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
            let role = Role::parse(parts.next().expect("a role")).expect("a known role");
            let branch = parts.next().expect("a branch");
            Sessions {
                owner: None,
                staff: Actor::staff("employee-7", role, BranchId::new(branch)),
            }
        })
    }
}

fn front_at(branch: &str) -> String {
    format!("staff:FRONT:{branch}")
}

// ------------------------------------------------------------ the commands

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct PairInput {
    first: String,
    second: String,
    tag: String,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Marked {
    tag: String,
}

/// Declares `Pair(first, second)` from its input and writes one mark on each.
struct WritePair;

impl CommandAction for WritePair {
    const NAME: &'static str = "test-write-pair";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Input = PairInput;
    type Output = Marked;
    const ROUTE: &'static str = "/api/test-pair";
    const METHOD: Method = Method::POST;

    fn locks(&self, _branch: &BranchId, input: &PairInput) -> Locks {
        Locks::Pair(
            BranchId::new(input.first.as_str()),
            BranchId::new(input.second.as_str()),
        )
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        input: PairInput,
        _key: IdempotencyKey,
    ) -> Result<Marked, V3Error> {
        for b in [&input.first, &input.second] {
            sqlx::query("INSERT INTO test_marks (branch_id, tag) VALUES ($1, $2)")
                .bind(b.as_str())
                .bind(input.tag.as_str())
                .execute(&mut **tx)
                .await
                .expect("mark through run's tx");
        }
        Ok(Marked { tag: input.tag })
    }
}

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct OneInput {
    tag: String,
}

/// Overrides nothing: the default `locks()`.
struct WriteOne;

impl CommandAction for WriteOne {
    const NAME: &'static str = "test-write-one";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Input = OneInput;
    type Output = Marked;
    const ROUTE: &'static str = "/api/test-one";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        _who: &Actor,
        input: OneInput,
        _key: IdempotencyKey,
    ) -> Result<Marked, V3Error> {
        sqlx::query("INSERT INTO test_marks (branch_id, tag) VALUES ($1, $2)")
            .bind(branch.as_str())
            .bind(input.tag.as_str())
            .execute(&mut **tx)
            .await
            .expect("mark through run's tx");
        Ok(Marked { tag: input.tag })
    }
}

struct Answer {
    status: StatusCode,
    body: Value,
    text: String,
}

async fn post(db: &TestDb, who: &str, route: &str, body: Value, key: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                CommandEndpoint::register(cfg, Arc::new(WritePair));
                CommandEndpoint::register(cfg, Arc::new(WriteOne));
                bootstrap::configure(cfg, state);
            })
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(route)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, body, text }
}

async fn pair(db: &TestDb, at: &str, first: &str, second: &str, tag: &str) -> Answer {
    post(
        db,
        &front_at(at),
        "/api/test-pair",
        json!({ "first": first, "second": second, "tag": tag }),
        tag,
    )
    .await
}

async fn within<T>(what: &str, fut: impl Future<Output = T>) -> T {
    tokio::time::timeout(MUST_FINISH, fut)
        .await
        .unwrap_or_else(|_| {
            panic!("{what}: did not finish within {MUST_FINISH:?} (a lock wait cycle?)")
        })
}

fn is_ok(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: status; body {}", a.text);
}

// ------------------------------------------------------------ the tests

/// Two opposite pairs at once, many times: X's command declares `Pair(X, Y)`, Y's declares `Pair(Y, X)`.
/// With one total lock order both always finish, and each run moves both versions by one.
#[actix_web::test]
async fn opposite_pairs_at_once_both_finish_and_move_both_versions() {
    let db = TestDb::start().await;
    let (x, y) = ("XC-B02", "XC-B03");
    for round in 0..ROUNDS {
        let (vx, vy) = (db.version(x).await, db.version(y).await);
        let (t1, t2) = (format!("xy-{round}"), format!("yx-{round}"));
        let (a, b) = within(&format!("round {round}"), async {
            tokio::join!(pair(&db, x, x, y, &t1), pair(&db, y, y, x, &t2),)
        })
        .await;
        is_ok(&a, &format!("round {round} X→Y"));
        is_ok(&b, &format!("round {round} Y→X"));
        assert_eq!(
            db.version(x).await,
            vx + 2,
            "round {round}: X moved once per pair"
        );
        assert_eq!(
            db.version(y).await,
            vy + 2,
            "round {round}: Y moved once per pair"
        );
    }
    assert_eq!(db.marks(x).await, 2 * ROUNDS as i64);
    assert_eq!(db.marks(y).await, 2 * ROUNDS as i64);
    db.finish().await;
}

/// The same with `MAIN`, whose lock comes first whatever the ids: `A-1` sorts before `MAIN`, so an order by
/// id alone would take `A-1` first for one command and `MAIN` first for the other.
#[actix_web::test]
async fn opposite_pairs_with_main_both_finish() {
    let db = TestDb::start().await;
    let (x, main) = ("A-1", "MAIN");
    assert!(x < main);
    for round in 0..ROUNDS {
        let (t1, t2) = (format!("xm-{round}"), format!("mx-{round}"));
        let (a, b) = within(&format!("round {round}"), async {
            tokio::join!(pair(&db, x, x, main, &t1), pair(&db, main, main, x, &t2),)
        })
        .await;
        is_ok(&a, &format!("round {round} A-1→MAIN"));
        is_ok(&b, &format!("round {round} MAIN→A-1"));
    }
    assert_eq!(db.version(x).await, 2 * ROUNDS as i64);
    assert_eq!(db.version(main).await, 2 * ROUNDS as i64);
    db.finish().await;
}

/// A pair the caller was never checked against is refused before anything is written: no idempotency
/// record, no mark, no version moved.
#[actix_web::test]
async fn a_pair_without_the_callers_branch_is_out_of_scope_and_writes_nothing() {
    let db = TestDb::start().await;
    let a = pair(&db, "XC-B03", "A-1", "XC-B02", "foreign").await;
    assert_eq!(a.status.as_u16(), 403, "status; body {}", a.text);
    assert_eq!(
        a.body["error"]["code"],
        json!("outOfScope"),
        "body {}",
        a.text
    );
    assert!(db.records().await.is_empty(), "no idempotency record");
    for b in ["A-1", "XC-B02", "XC-B03"] {
        assert_eq!(db.marks(b).await, 0, "{b}: nothing written");
        assert_eq!(db.version(b).await, 0, "{b}: version not moved");
    }
    db.finish().await;
}

/// `Pair(b, b)` is `One(b)`: the branch moves once, not twice.
#[actix_web::test]
async fn a_pair_of_one_branch_moves_it_once() {
    let db = TestDb::start().await;
    is_ok(
        &pair(&db, "XC-B02", "XC-B02", "XC-B02", "same").await,
        "Pair(X, X)",
    );
    assert_eq!(db.version("XC-B02").await, 1);
    assert_eq!(
        db.marks("XC-B02").await,
        2,
        "run wrote both marks through one tx"
    );
    db.finish().await;
}

/// A command that overrides nothing keeps `One(branch)`: only the caller's branch moves.
#[actix_web::test]
async fn the_default_locks_only_the_callers_branch() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        &front_at("XC-B02"),
        "/api/test-one",
        json!({ "tag": "one" }),
        "one",
    )
    .await;
    is_ok(&a, "default locks");
    assert_eq!(db.version("XC-B02").await, 1);
    assert_eq!(db.version("XC-B03").await, 0);
    assert_eq!(db.marks("XC-B02").await, 1);
    db.finish().await;
}
