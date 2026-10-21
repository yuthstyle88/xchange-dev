//! A "create branch" request sent from a branch other than `MAIN`, and the
//! screen giving that same request up, at the same moment (ticket #117).
//!
//! A request id is applied or given up, never both. `BranchService::create`
//! reads whether the request was given up **in the branch it was sent from**,
//! and `AbandonRequest::abandon` reads whether a branch was created with it —
//! each must wait for the other, so `create` holds the lock of the sending
//! branch too (`MAIN` first, then that branch).
//!
//! The first two tests hold one side still inside its transaction with a row
//! lock taken by the test, and read from the database who waits for whom
//! (`pg_blocking_pids`) before they go on: the order is established, never
//! assumed after a pause, so a slow machine changes nothing. The third lets
//! the two race freely for many rounds.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Barrier;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;
use xc_rate::domain::branch::commands::create_branch::CreateBranch;
use xc_rate::domain::branch::entities::branch::Branch;
use xchange::application::compat::abandon_request::{AbandonRequest, RequestResolution};

/// Upper bound for anything that must finish.
const MUST_FINISH: Duration = Duration::from_secs(30);
/// How often the database is asked who waits for whom.
const ASK_AGAIN: Duration = Duration::from_millis(10);

const ACTOR: &str = "owner";
const MAIN: &str = "MAIN";
const OTHER: &str = "B2";
const ABANDONED: &str = "คำขอนี้ถูกยกเลิกแล้ว กรุณาโหลดข้อมูลล่าสุด";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// A fresh request id of the shape the services accept.
fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        SEQ.fetch_add(1, Ordering::Relaxed)
    })
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap()
}

async fn within<T>(what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(MUST_FINISH, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {MUST_FINISH:?}: {what}"),
    }
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied, and a
    /// second branch `B2` beside `MAIN`.
    async fn start() -> Arc<TestDb> {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The create-branch race tests need a real PostgreSQL and do not skip. Point \
                 it at an existing database whose name ends in `_test`, e.g.\n\
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

        let schema = unique("race117");
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

        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(OTHER)
            .bind("Second branch")
            .execute(&pool)
            .await
            .expect("insert the second branch");
        // Both branches have been written before, as on a server that has
        // lived: their version rows exist and are committed. A lock that was
        // only the creation of such a row would otherwise look like a lock.
        sqlx::query("INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0), ($2, 0)")
            .bind(MAIN)
            .bind(OTHER)
            .execute(&pool)
            .await
            .expect("insert the version rows");

        Arc::new(TestDb { pool, schema })
    }

    fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    /// `adminCreateBranch` with request `id`, sent from `from`.
    async fn create(&self, from: &str, id: &str) -> AppResult<Branch> {
        BranchService::new(self.branch_tx())
            .create(
                CreateBranch {
                    request_id: id.to_owned(),
                    name: format!("Branch of {id}"),
                    from_branch: BranchId::new(from),
                },
                ACTOR,
                now(),
            )
            .await
    }

    /// `adminAbandonRequest` with `{requestId: id}` in `branch`.
    async fn abandon(&self, branch: &str, id: &str) -> AppResult<RequestResolution> {
        AbandonRequest::new(self.branch_tx())
            .abandon(
                &BranchId::new(branch),
                &json!({ "requestId": id }),
                ACTOR,
                now(),
            )
            .await
    }

    async fn branch_created_with(&self, id: &str) -> bool {
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM branches WHERE request_id = $1)")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read branches")
    }

    async fn given_up(&self, branch: &str, id: &str) -> bool {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM abandoned_requests WHERE branch_id = $1 AND request_id = $2)",
        )
        .bind(branch)
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("read abandoned_requests")
    }

    /// The backends that wait for a lock `holder` holds, as the database
    /// sees it now.
    async fn blocked_by(&self, holder: i32) -> Vec<i32> {
        sqlx::query_scalar(
            "SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) ORDER BY pid",
        )
        .bind(holder)
        .fetch_all(&self.pool)
        .await
        .expect("read pg_stat_activity")
    }

    /// Waits until exactly one backend waits for `holder` and gives its pid.
    /// `call` is the call that must be that backend: when it ends instead,
    /// it did not wait, and the test fails with `otherwise`.
    async fn the_one_waiting_for<T: std::fmt::Debug>(
        &self,
        holder: i32,
        call: &mut tokio::task::JoinHandle<T>,
        otherwise: &str,
    ) -> i32 {
        let started = std::time::Instant::now();
        loop {
            if let [pid] = self.blocked_by(holder).await.as_slice() {
                return *pid;
            }
            if call.is_finished() {
                panic!("{otherwise}: {:?}", call.await);
            }
            assert!(
                started.elapsed() < MUST_FINISH,
                "nobody came to wait for backend {holder} within {MUST_FINISH:?}"
            );
            tokio::time::sleep(ASK_AGAIN).await;
        }
    }

    async fn version(&self, branch: &str) -> i64 {
        self.branch_tx()
            .version(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    /// Remove the branch a round created, so that the next round is not the
    /// twenty-first branch.
    async fn forget_branch_of(&self, id: &str) {
        sqlx::query("DELETE FROM branches WHERE request_id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("delete the round's branch");
    }

    async fn finish(self: Arc<Self>) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked at start");
        let options = PgConnectOptions::from_str(&url).expect("checked at start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("connect to drop the schema");
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

/// What the two calls of one round answered and what the database holds, as
/// one of the two outcomes a request id may have — or a panic.
fn exactly_one_wins(
    here: &str,
    created: &AppResult<Branch>,
    resolved: &AppResult<RequestResolution>,
    branch_exists: bool,
    row_exists: bool,
) {
    match (created, resolved) {
        // applied: the branch is there, the screen is told so, nothing is given up
        (Ok(_), Ok(RequestResolution::Applied)) => {
            assert!(branch_exists, "{here}: answered applied, but no branch");
            assert!(
                !row_exists,
                "{here}: applied, and a row in abandoned_requests all the same"
            );
        }
        // given up: the create is refused with the sentence, no branch
        (Err(AppError::Domain(sentence)), Ok(RequestResolution::Cancelled))
            if sentence == ABANDONED =>
        {
            assert!(!branch_exists, "{here}: given up, and the branch exists");
            assert!(row_exists, "{here}: answered cancelled, but no row");
        }
        other => panic!(
            "{here}: the request is both given up and applied, or neither: {other:?} \
             (branch exists: {branch_exists}, abandoned row: {row_exists})"
        ),
    }
}

/// `create` has read "not given up" and waits to insert its branch; the
/// abandon of the same id in the sending branch must wait for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_abandon_waits_for_a_create_that_is_under_way() {
    let db = TestDb::start().await;
    let id = request_id();

    // Hold `create` inside its transaction, after its checks: another
    // transaction owns the primary key its insert needs.
    let mut holder = db.pool.begin().await.expect("begin");
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
        .bind(format!("B{id}"))
        .bind(format!("held for {id}"))
        .execute(&mut *holder)
        .await
        .expect("hold the new branch's id");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("the holder's backend");

    let mut create = tokio::spawn({
        let (db, id) = (db.clone(), id.clone());
        async move { db.create(OTHER, &id).await }
    });
    // the create has done its checks and waits for the test at its insert
    let create_pid = db
        .the_one_waiting_for(
            holder_pid,
            &mut create,
            "the create did not reach its insert",
        )
        .await;

    let mut abandon = tokio::spawn({
        let (db, id) = (db.clone(), id.clone());
        async move { db.abandon(OTHER, &id).await }
    });
    // … and the abandon waits for the create: the database says so
    db.the_one_waiting_for(
        create_pid,
        &mut abandon,
        "the abandon answered while a create of the same request was under way",
    )
    .await;
    assert!(!create.is_finished() && !abandon.is_finished());

    holder.rollback().await.expect("let the create go on");
    let created = within("the create", create).await.expect("join");
    let resolved = within("the abandon", abandon).await.expect("join");
    exactly_one_wins(
        "create first",
        &created,
        &resolved,
        db.branch_created_with(&id).await,
        db.given_up(OTHER, &id).await,
    );
    assert!(created.is_ok(), "the create was first: {created:?}");

    db.finish().await;
}

/// The abandon has read "not applied" and waits to write its row; a create
/// with the same id sent from that branch must wait for it, and is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_create_waits_for_an_abandon_that_is_under_way() {
    let db = TestDb::start().await;
    let id = request_id();

    // Hold the abandon inside its transaction, after its check: another
    // transaction owns the row it is about to write.
    let mut holder = db.pool.begin().await.expect("begin");
    sqlx::query(
        "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) VALUES ($1, $2, $3, $4)",
    )
    .bind(OTHER)
    .bind(&id)
    .bind(now())
    .bind("the test")
    .execute(&mut *holder)
    .await
    .expect("hold the abandoned row");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("the holder's backend");

    let mut abandon = tokio::spawn({
        let (db, id) = (db.clone(), id.clone());
        async move { db.abandon(OTHER, &id).await }
    });
    // the abandon has done its check and waits for the test at its insert
    let abandon_pid = db
        .the_one_waiting_for(
            holder_pid,
            &mut abandon,
            "the abandon did not reach its insert",
        )
        .await;

    let mut create = tokio::spawn({
        let (db, id) = (db.clone(), id.clone());
        async move { db.create(OTHER, &id).await }
    });
    // … and the create waits for the abandon: the database says so
    db.the_one_waiting_for(
        abandon_pid,
        &mut create,
        "the create answered while the same request was being given up",
    )
    .await;
    assert!(!create.is_finished() && !abandon.is_finished());

    holder.rollback().await.expect("let the abandon go on");
    let resolved = within("the abandon", abandon).await.expect("join");
    let created = within("the create", create).await.expect("join");
    exactly_one_wins(
        "abandon first",
        &created,
        &resolved,
        db.branch_created_with(&id).await,
        db.given_up(OTHER, &id).await,
    );
    assert!(created.is_err(), "the abandon was first: {created:?}");

    db.finish().await;
}

/// Many rounds, nobody held: whichever comes first, one wins.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn in_every_round_of_a_free_race_exactly_one_wins() {
    let db = TestDb::start().await;
    let (mut applied, mut given_up) = (0, 0);

    for round in 0..60 {
        let id = request_id();
        let barrier = Arc::new(Barrier::new(2));
        let create = tokio::spawn({
            let (db, id, barrier) = (db.clone(), id.clone(), barrier.clone());
            async move {
                barrier.wait().await;
                db.create(OTHER, &id).await
            }
        });
        let abandon = tokio::spawn({
            let (db, id, barrier) = (db.clone(), id.clone(), barrier.clone());
            async move {
                barrier.wait().await;
                // every other round the abandon gets a head start
                if round % 2 == 0 {
                    tokio::task::yield_now().await;
                }
                db.abandon(OTHER, &id).await
            }
        });
        let created = within("the create", create).await.expect("join");
        let resolved = within("the abandon", abandon).await.expect("join");
        exactly_one_wins(
            &format!("round {round}"),
            &created,
            &resolved,
            db.branch_created_with(&id).await,
            db.given_up(OTHER, &id).await,
        );
        if created.is_ok() {
            applied += 1;
        } else {
            given_up += 1;
        }
        db.forget_branch_of(&id).await;
    }
    assert_eq!(applied + given_up, 60);

    db.finish().await;
}

/// The lock of the sending branch is a real branch write: its version moves
/// with the create, and only when the create is accepted. A create sent from
/// `MAIN` takes one lock, as before.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_create_moves_the_version_of_the_branch_it_was_sent_from() {
    let db = TestDb::start().await;

    let (main, other) = (db.version(MAIN).await, db.version(OTHER).await);
    db.create(OTHER, &request_id())
        .await
        .expect("a create from the second branch");
    assert_eq!(
        (db.version(MAIN).await, db.version(OTHER).await),
        (main + 1, other + 1),
        "sent from the second branch: both versions move by one"
    );

    db.create(MAIN, &request_id())
        .await
        .expect("a create from MAIN");
    assert_eq!(
        (db.version(MAIN).await, db.version(OTHER).await),
        (main + 2, other + 1),
        "sent from MAIN: MAIN alone, and by one"
    );

    // a refused create moves neither
    let id = request_id();
    assert_eq!(
        db.abandon(OTHER, &id).await.expect("give the id up"),
        RequestResolution::Cancelled
    );
    let (main, other) = (db.version(MAIN).await, db.version(OTHER).await);
    let refused = db.create(OTHER, &id).await;
    assert!(
        matches!(&refused, Err(AppError::Domain(sentence)) if sentence == ABANDONED),
        "{refused:?}"
    );
    assert_eq!(
        (db.version(MAIN).await, db.version(OTHER).await),
        (main, other)
    );

    db.finish().await;
}
