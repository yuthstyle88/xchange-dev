//! Written before the implementation (ticket T11d). Do not edit, delete or weaken.
//!
//! CA-10 (#626, #2955): v2's shift path is deleted. The shift writes that apply a request are written straight
//! into the tables (setup only). Retired with it: the two fixture replays (each holds v2's `adminOpenShift` refusing
//! an id, #37 / #149), and the tests whose subject or an assertion is v2's open / move / close answering an id
//! (`a_cancelled_request_is_refused_by_every_write_of_the_branch`, `an_applied_open_shift_request_is_applied`,
//! `the_same_id_given_up_in_one_branch_is_free_in_another`, `abandoning_and_opening_a_shift_at_once_exactly_one_wins`,
//! and in round 4 `sixteen_and_sixty_characters_are_accepted_and_the_id_is_trimmed`, whose second half asserts v2's
//! open refusing an abandoned id).
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
//!
//! Re-expected for RE-7 #604 by the ticket author (design `docs/design/blueprint/rates.md` §8
//! migration 6, §9): `RateService::save_rate` and `adminSaveRate` are gone, so a rate is no
//! longer a write that carries a request id. Where a test applied a request with a rate, it
//! applies it with another write of the branch (an open shift); a recorded `adminSaveRate` step
//! is a known difference (design §9 removes the action): it is answered `RETIRED` and saves
//! nothing, so a later abandon of its id is `CANCELLED` where v2 answered `APPLIED`. The two
//! tests about the rate save itself are removed with it (see the notes where they stood).

mod compat_common;

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::{Barrier, Semaphore, oneshot};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xc_rate::domain::branch::commands::create_branch::CreateBranch;
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

    /// CA-10 (#626, #2955): v2's `ShiftService::open` is deleted with the core. An OPEN shift that applied
    /// request `id`, written straight into the tables in one branch transaction (setup only).
    async fn open_shift(&self, branch_id: &str, id: &str) -> AppResult<Seeded> {
        let (b, id) = (branch(branch_id), id.to_owned());
        let inner = b.clone();
        within(
            "open a shift",
            self.branch_tx().run(&b, move |tx| {
                Box::pin(async move { seed_shift_in(tx, &inner, &id).await })
            }),
        )
        .await
        .map(|id| Seeded { id })
    }

    /// CA-10: v2's `move_stock` with request `id` on `shift`, written straight in (setup only).
    async fn move_stock(&self, branch_id: &str, shift: &str, id: &str) {
        sqlx::query(
            "INSERT INTO stock_movements (id, branch_id, shift_id, code, direction, amount, reason, request_id, at, by) \
             VALUES ($1, $2, $3, 'USD', 'IN', 5, 'top up', $4, $5, $6)",
        )
        .bind(format!("MV-{}", uuid::Uuid::new_v4()))
        .bind(branch_id)
        .bind(shift)
        .bind(id)
        .bind(at(1))
        .bind(ACTOR)
        .execute(&self.pool)
        .await
        .expect("seed a movement");
    }

    /// CA-10: v2's `close` with request `id` of `shift`, written straight in (setup only).
    async fn close_shift(&self, shift: &str, id: &str) {
        sqlx::query(
            "UPDATE shifts SET status = 'CLOSED', closed_at = $1, closed_by = $2, close_note = '', \
             close_request_id = $3 WHERE id = $4",
        )
        .bind(at(2))
        .bind(ACTOR)
        .bind(id)
        .bind(shift)
        .execute(&self.pool)
        .await
        .expect("seed a close");
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

fn pair(branch_id: &str, id: &str) -> (String, String) {
    (branch_id.to_string(), id.to_string())
}

// ---------------------------------------------------------------------------
// The recorded steps
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// APPLIED
// ---------------------------------------------------------------------------

// Re-expected for RE-7 #604 by the ticket author: `an_applied_rate_request_is_applied_and_no_row_
// is_written` is removed. It applied a request by `RateService::save_rate`, which RE-7 removes
// (design §9); no rate carries a request id any more. That a recorded rate save's id is then
// CANCELLED is held by `engine_actions_fixture_145_to_152_answer_as_v2_did` (#150).

#[tokio::test(flavor = "multi_thread")]
async fn applied_stock_movement_and_close_shift_requests_are_applied() {
    let db = TestDb::start().await;
    let shift = db.open_shift(MAIN, &request_id()).await.unwrap();

    let moved = request_id();
    db.move_stock(MAIN, &shift.id, &moved).await;
    assert_eq!(db.abandon(MAIN, &moved, 2).await.unwrap(), Applied);

    let closed = request_id();
    db.close_shift(&shift.id, &closed).await;
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

    // an open shift, a movement and a closed shift — all in B2 (re-expected for RE-7 #604: a
    // rate is no longer a write that carries a request id)
    let opened = request_id();
    let shift = db.open_shift("B2", &opened).await.unwrap();
    let moved = request_id();
    db.move_stock("B2", &shift.id, &moved).await;
    let closed = request_id();
    db.close_shift(&shift.id, &closed).await;

    let ids = [&opened, &moved, &closed];
    for id in ids {
        assert_eq!(db.abandon(MAIN, id, 3).await.unwrap(), Cancelled, "in MAIN");
        assert_eq!(db.abandon("B2", id, 4).await.unwrap(), Applied, "in B2");
    }
    let mut expected: Vec<_> = ids.iter().map(|id| pair(MAIN, id)).collect();
    expected.sort();
    assert_eq!(db.keys().await, expected);
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
    let id = id.to_owned();
    let holder = tokio::spawn(async move {
        let b = branch(branch_id);
        let inner = b.clone();
        branch_tx
            .run(&b, move |tx| {
                Box::pin(async move {
                    // CA-10: v2's `open_in` is deleted; the same OPEN shift row, in this held transaction
                    seed_shift_in(tx, &inner, &id).await?;
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

// Re-expected for RE-7 #604 by the ticket author: `abandoning_and_saving_a_rate_at_once_exactly_
// one_wins` is removed with `RateService::save_rate` (design §9). The same race against the
// writes that remain is held by the tests below.

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

/// A shift the setup wrote (CA-10: v2's `Shift` answer is gone with `ShiftService::open`).
#[derive(Debug)]
struct Seeded {
    id: String,
}

/// CA-10 (#626, #2955): the OPEN shift row v2's `open_in` wrote for request `id` in `branch` (opener `ACTOR`,
/// USD 10), written in `tx`. Answers the shift's id.
async fn seed_shift_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    id: &str,
) -> AppResult<String> {
    let shift = format!("SH-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
         SELECT $1, $2, 'OPEN', $3, $4, '', $5, u.id FROM users u WHERE u.legacy_employee_id IS NULL AND EXISTS \
         (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at LIMIT 1",
    )
    .bind(&shift)
    .bind(branch.as_str())
    .bind(at(0))
    .bind(ACTOR)
    .bind(id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO shift_balances (shift_id, code, opening, expected) VALUES ($1, 'USD', 10, 10)",
    )
    .bind(&shift)
    .execute(&mut **tx)
    .await?;
    Ok(shift)
}
