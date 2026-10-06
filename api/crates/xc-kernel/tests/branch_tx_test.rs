//! Written before the implementation (ticket T2d). Do not edit, delete or weaken.
//!
//! Specification of `xc_kernel::branch_tx::BranchTx`, the per-branch database
//! lock of design §4 ("Concurrency"): one transaction that takes the branch's
//! `branch_versions` row `FOR UPDATE`, runs the closure, raises `version` by
//! one and commits.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an
//! existing database whose name ends in `_test`, for example
//! `postgres://postgres:postgres@localhost:5432/xchange_test`. When the
//! variable is absent every test here FAILS; none of them skips. A locking
//! test that passes without touching a database is a gate that cannot fail.
//!
//! Each test works in a schema of its own (`search_path` is set on every
//! connection of its pool) and applies `api/migrations/` there, so the tests
//! run in parallel and the migration is exercised from an empty schema each
//! time. The migration and the implementation must therefore name
//! `branch_versions` without a schema prefix.
//!
//! Every wait has a bound: a wrong implementation fails, it does not hang.

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::{Barrier, oneshot};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

/// Upper bound for anything that must finish.
const MUST_FINISH: Duration = Duration::from_secs(15);
/// How long a second writer is watched to prove it stays outside.
const STAYS_OUT: Duration = Duration::from_millis(400);

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

fn branch(label: &str) -> BranchId {
    BranchId::new(unique(label))
}

/// Await `fut`, failing the test when it takes longer than `limit`.
async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {limit:?}: {what}"),
    }
}

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The BranchTx tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/xchange_test"
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

        let schema = unique("t2d");
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
            .max_connections(12)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        // Tables owned by this test file: something for a closure to write.
        sqlx::query("CREATE TABLE t2d_notes (branch_id text NOT NULL, note text NOT NULL)")
            .execute(&pool)
            .await
            .expect("create t2d_notes");
        sqlx::query("CREATE TABLE t2d_counters (branch_id text PRIMARY KEY, n bigint NOT NULL)")
            .execute(&pool)
            .await
            .expect("create t2d_counters");

        Self { pool, schema }
    }

    fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    /// Rows in `t2d_notes` for this branch, as committed.
    async fn notes(&self, branch: &BranchId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM t2d_notes WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .expect("count notes")
    }

    async fn seed_counter(&self, branch: &BranchId) {
        sqlx::query("INSERT INTO t2d_counters (branch_id, n) VALUES ($1, 0)")
            .bind(branch.as_str())
            .execute(&self.pool)
            .await
            .expect("seed counter");
    }

    async fn counter(&self, branch: &BranchId) -> i64 {
        sqlx::query_scalar("SELECT n FROM t2d_counters WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .expect("read counter")
    }

    /// Rows in `branch_versions` for this branch, as committed.
    async fn version_rows(&self, branch: &BranchId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM branch_versions WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .expect("count branch_versions rows")
    }

    /// Drop the schema. Reached only by a test that passed; a failed test
    /// leaves its schema behind for inspection.
    async fn finish(self) {
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

async fn write_note(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    note: &str,
) -> AppResult<()> {
    sqlx::query("INSERT INTO t2d_notes (branch_id, note) VALUES ($1, $2)")
        .bind(branch.as_str())
        .bind(note)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The first half of a read-modify-write that is only correct under the lock.
async fn read_counter(tx: &mut Transaction<'_, Postgres>, branch: &BranchId) -> AppResult<i64> {
    let n = sqlx::query_scalar("SELECT n FROM t2d_counters WHERE branch_id = $1")
        .bind(branch.as_str())
        .fetch_one(&mut **tx)
        .await?;
    Ok(n)
}

/// The second half: writes back what was read, plus one.
async fn store_counter(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    n: i64,
) -> AppResult<()> {
    sqlx::query("UPDATE t2d_counters SET n = $2 WHERE branch_id = $1")
        .bind(branch.as_str())
        .bind(n)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// One committed `run` that writes nothing, so the branch has its row and the
/// next `run` takes the lock on an existing row.
async fn touch(bt: &BranchTx, branch: &BranchId) {
    within(
        MUST_FINISH,
        "a run that writes nothing",
        bt.run(branch, |_tx| Box::pin(async move { Ok(()) })),
    )
    .await
    .expect("a run that writes nothing commits");
}

async fn version(bt: &BranchTx, branch: &BranchId) -> i64 {
    within(MUST_FINISH, "BranchTx::version", bt.version(branch))
        .await
        .expect("BranchTx::version")
}

// ---------------------------------------------------------------------------
// (a) commit
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_commits_the_closures_writes_and_raises_the_version_by_exactly_one() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("commit");

    assert_eq!(version(&bt, &b).await, 0, "a branch never written is at 0");

    let inner = b.clone();
    let out = within(
        MUST_FINISH,
        "first run",
        bt.run(&b, move |tx| {
            Box::pin(async move {
                write_note(tx, &inner, "first").await?;
                Ok(42_u32)
            })
        }),
    )
    .await;
    assert!(
        matches!(out, Ok(42)),
        "run returns the closure's value: {out:?}"
    );
    assert_eq!(db.notes(&b).await, 1, "the closure's write is committed");
    assert_eq!(version(&bt, &b).await, 1, "first run: 0 -> 1");

    let inner = b.clone();
    within(
        MUST_FINISH,
        "second run",
        bt.run(&b, move |tx| {
            Box::pin(async move { write_note(tx, &inner, "second").await })
        }),
    )
    .await
    .expect("second run commits");
    assert_eq!(db.notes(&b).await, 2);
    assert_eq!(version(&bt, &b).await, 2, "second run: 1 -> 2");

    // A run that writes nothing is still one write transaction.
    touch(&bt, &b).await;
    assert_eq!(version(&bt, &b).await, 3, "third run: 2 -> 3");
    assert_eq!(
        db.version_rows(&b).await,
        1,
        "one branch_versions row per branch"
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// (b) rollback
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closure_error_rolls_back_its_writes_and_leaves_the_version_unchanged() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("rollback");

    touch(&bt, &b).await;
    assert_eq!(version(&bt, &b).await, 1);

    let inner = b.clone();
    let out: AppResult<()> = within(
        MUST_FINISH,
        "failing run",
        bt.run(&b, move |tx| {
            Box::pin(async move {
                write_note(tx, &inner, "must not survive").await?;
                Err(AppError::Domain("stock guard".to_owned()))
            })
        }),
    )
    .await;
    match out {
        Err(AppError::Domain(message)) => assert_eq!(
            message, "stock guard",
            "the closure's own error comes back unchanged"
        ),
        other => panic!("expected the closure's AppError::Domain, got {other:?}"),
    }
    assert_eq!(db.notes(&b).await, 0, "the failed closure's write is gone");
    assert_eq!(
        version(&bt, &b).await,
        1,
        "a failed run does not move the version"
    );

    // The branch is usable afterwards and the next good run is 1 -> 2.
    touch(&bt, &b).await;
    assert_eq!(version(&bt, &b).await, 2);

    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closure_error_on_the_first_use_of_a_branch_leaves_it_at_version_zero() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("rollback_first");

    let inner = b.clone();
    let out: AppResult<()> = within(
        MUST_FINISH,
        "failing first run",
        bt.run(&b, move |tx| {
            Box::pin(async move {
                write_note(tx, &inner, "must not survive").await?;
                Err(AppError::Conflict("refused".to_owned()))
            })
        }),
    )
    .await;
    assert!(
        matches!(&out, Err(AppError::Conflict(m)) if m == "refused"),
        "the closure's own error comes back unchanged: {out:?}"
    );
    assert_eq!(db.notes(&b).await, 0);
    assert_eq!(version(&bt, &b).await, 0);

    touch(&bt, &b).await;
    assert_eq!(
        version(&bt, &b).await,
        1,
        "the first run that commits is 0 -> 1"
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// (c) one branch: serial
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_run_on_the_same_branch_waits_until_the_first_has_committed() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("serial");
    db.seed_counter(&b).await;
    touch(&bt, &b).await; // the row exists: the lock is on an existing row
    assert_eq!(version(&bt, &b).await, 1);

    let (a_entered_tx, a_entered_rx) = oneshot::channel::<()>();
    let (release_a_tx, release_a_rx) = oneshot::channel::<()>();
    let b_entered = Arc::new(AtomicBool::new(false));

    let first = {
        let (bt, b) = (bt.clone(), b.clone());
        tokio::spawn(async move {
            let inner = b.clone();
            bt.run(&b, move |tx| {
                Box::pin(async move {
                    let n = read_counter(tx, &inner).await?;
                    let _ = a_entered_tx.send(());
                    match tokio::time::timeout(MUST_FINISH, release_a_rx).await {
                        Ok(Ok(())) => {}
                        _ => return Err(AppError::Internal("A was never released".to_owned())),
                    }
                    store_counter(tx, &inner, n + 1).await
                })
            })
            .await
        })
    };

    within(MUST_FINISH, "A enters its closure", a_entered_rx)
        .await
        .expect("A entered its closure");

    let second = {
        let (bt, b, b_entered) = (bt.clone(), b.clone(), b_entered.clone());
        tokio::spawn(async move {
            let inner = b.clone();
            bt.run(&b, move |tx| {
                Box::pin(async move {
                    b_entered.store(true, Ordering::SeqCst);
                    let n = read_counter(tx, &inner).await?;
                    store_counter(tx, &inner, n + 1).await
                })
            })
            .await
        })
    };

    tokio::time::sleep(STAYS_OUT).await;
    assert!(
        !b_entered.load(Ordering::SeqCst),
        "B's closure started while A was still inside its closure on the same branch"
    );
    assert_eq!(
        version(&bt, &b).await,
        1,
        "nothing has committed while A is inside"
    );

    release_a_tx.send(()).expect("A is still waiting");
    within(MUST_FINISH, "A finishes", first)
        .await
        .expect("A did not panic")
        .expect("A commits");
    within(MUST_FINISH, "B finishes after A", second)
        .await
        .expect("B did not panic")
        .expect("B commits");

    assert!(b_entered.load(Ordering::SeqCst));
    assert_eq!(
        db.counter(&b).await,
        2,
        "B read the counter after A wrote it: no lost update"
    );
    assert_eq!(
        version(&bt, &b).await,
        3,
        "two runs: 1 -> 2 -> 3, distinct and consecutive"
    );

    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_concurrent_runs_on_one_branch_are_serial_and_lose_no_update() {
    const TASKS: usize = 8;

    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("crowd");
    db.seed_counter(&b).await;
    touch(&bt, &b).await;

    let start = Arc::new(Barrier::new(TASKS));
    let inside = Arc::new(AtomicUsize::new(0));
    let most_inside = Arc::new(AtomicUsize::new(0));

    let mut tasks = Vec::new();
    for _ in 0..TASKS {
        let (bt, b) = (bt.clone(), b.clone());
        let (start, inside, most_inside) = (start.clone(), inside.clone(), most_inside.clone());
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            let inner = b.clone();
            bt.run(&b, move |tx| {
                Box::pin(async move {
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    most_inside.fetch_max(now, Ordering::SeqCst);
                    let n = read_counter(tx, &inner).await?;
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    let stored = store_counter(tx, &inner, n + 1).await;
                    inside.fetch_sub(1, Ordering::SeqCst);
                    stored
                })
            })
            .await
        }));
    }
    for (i, task) in tasks.into_iter().enumerate() {
        within(MUST_FINISH, "a crowd task finishes", task)
            .await
            .unwrap_or_else(|e| panic!("task {i} panicked: {e}"))
            .unwrap_or_else(|e| panic!("task {i} failed: {e}"));
    }

    assert_eq!(
        most_inside.load(Ordering::SeqCst),
        1,
        "more than one closure was inside the same branch at once"
    );
    assert_eq!(db.counter(&b).await, TASKS as i64, "no update was lost");
    assert_eq!(
        version(&bt, &b).await,
        1 + TASKS as i64,
        "one version per committed run"
    );

    db.finish().await;
}

// ---------------------------------------------------------------------------
// (d) two branches: independent
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runs_on_different_branches_are_inside_their_closures_at_the_same_time() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let east = branch("east");
    let west = branch("west");
    touch(&bt, &east).await;
    touch(&bt, &west).await;

    // Both closures must stand at this barrier together. Under a lock wider
    // than one branch the second never arrives and the first times out.
    let meet = Arc::new(Barrier::new(2));

    let spawn_run = |b: BranchId| {
        let (bt, meet) = (bt.clone(), meet.clone());
        tokio::spawn(async move {
            let inner = b.clone();
            bt.run(&b, move |tx| {
                Box::pin(async move {
                    write_note(tx, &inner, "inside").await?;
                    match tokio::time::timeout(Duration::from_secs(5), meet.wait()).await {
                        Ok(_) => Ok(()),
                        Err(_) => Err(AppError::Internal(format!(
                            "{inner} waited 5s inside its closure and the other branch never \
                             entered: the lock is wider than one branch"
                        ))),
                    }
                })
            })
            .await
        })
    };
    let first = spawn_run(east.clone());
    let second = spawn_run(west.clone());

    within(MUST_FINISH, "east finishes", first)
        .await
        .expect("east did not panic")
        .expect("east commits without waiting for west");
    within(MUST_FINISH, "west finishes", second)
        .await
        .expect("west did not panic")
        .expect("west commits without waiting for east");

    assert_eq!(version(&bt, &east).await, 2);
    assert_eq!(version(&bt, &west).await, 2);
    assert_eq!(db.notes(&east).await, 1);
    assert_eq!(db.notes(&west).await, 1);

    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_version_is_per_branch_and_a_run_moves_only_its_own() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let east = branch("east");
    let west = branch("west");

    touch(&bt, &east).await;
    touch(&bt, &east).await;
    touch(&bt, &west).await;

    assert_eq!(version(&bt, &east).await, 2);
    assert_eq!(version(&bt, &west).await, 1);

    db.finish().await;
}

// ---------------------------------------------------------------------------
// (e) first use
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_uses_of_a_new_branch_all_succeed_and_count_every_run() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();

    // Two at once, on several fresh branches, to meet the creation race.
    for round in 0..8 {
        let b = branch("fresh_pair");
        db.seed_counter(&b).await;
        let start = Arc::new(Barrier::new(2));
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let (bt, b, start) = (bt.clone(), b.clone(), start.clone());
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                let inner = b.clone();
                bt.run(&b, move |tx| {
                    Box::pin(async move {
                        let n = read_counter(tx, &inner).await?;
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        store_counter(tx, &inner, n + 1).await
                    })
                })
                .await
            }));
        }
        for task in tasks {
            within(MUST_FINISH, "a first use finishes", task)
                .await
                .expect("a first use did not panic")
                .unwrap_or_else(|e| panic!("round {round}: a first use failed: {e}"));
        }
        assert_eq!(
            version(&bt, &b).await,
            2,
            "round {round}: both first uses counted"
        );
        assert_eq!(
            db.counter(&b).await,
            2,
            "round {round}: the two first uses were serial"
        );
        assert_eq!(db.version_rows(&b).await, 1, "round {round}: one row");
    }

    // Six at once on one fresh branch.
    let b = branch("fresh_crowd");
    let start = Arc::new(Barrier::new(6));
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let (bt, b, start) = (bt.clone(), b.clone(), start.clone());
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            bt.run(&b, |_tx| Box::pin(async move { Ok(()) })).await
        }));
    }
    for task in tasks {
        within(MUST_FINISH, "a first use finishes", task)
            .await
            .expect("a first use did not panic")
            .expect("a first use commits");
    }
    assert_eq!(version(&bt, &b).await, 6);
    assert_eq!(db.version_rows(&b).await, 1);

    db.finish().await;
}

// ---------------------------------------------------------------------------
// (f) version()
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn version_of_a_branch_never_written_is_zero_and_reading_it_writes_nothing() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("unknown");

    let out = within(MUST_FINISH, "BranchTx::version", bt.version(&b)).await;
    assert!(
        matches!(out, Ok(0)),
        "an unknown branch is version 0, not an error: {out:?}"
    );
    assert_eq!(version(&bt, &b).await, 0, "and it stays 0 when asked again");
    assert_eq!(
        db.version_rows(&b).await,
        0,
        "version() is a read: it creates no branch_versions row"
    );

    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn version_shows_the_last_commit_and_does_not_wait_for_a_run_in_progress() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("read_during_write");
    touch(&bt, &b).await;

    let (entered_tx, entered_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();
    let writer = {
        let (bt, b) = (bt.clone(), b.clone());
        tokio::spawn(async move {
            bt.run(&b, move |_tx| {
                Box::pin(async move {
                    let _ = entered_tx.send(());
                    match tokio::time::timeout(MUST_FINISH, release_rx).await {
                        Ok(Ok(())) => Ok(()),
                        _ => Err(AppError::Internal(
                            "the writer was never released".to_owned(),
                        )),
                    }
                })
            })
            .await
        })
    };
    within(MUST_FINISH, "the writer enters its closure", entered_rx)
        .await
        .expect("the writer entered its closure");

    let seen = within(
        Duration::from_secs(3),
        "version() while a run holds the branch — a read must not queue behind the lock",
        bt.version(&b),
    )
    .await
    .expect("BranchTx::version");
    assert_eq!(seen, 1, "an uncommitted run is not visible in the version");

    release_tx.send(()).expect("the writer is still waiting");
    within(MUST_FINISH, "the writer finishes", writer)
        .await
        .expect("the writer did not panic")
        .expect("the writer commits");
    assert_eq!(version(&bt, &b).await, 2);

    db.finish().await;
}

// ---------------------------------------------------------------------------
// (g) the lock does not outlive a closure that never returns
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_panic_inside_the_closure_releases_the_branch_and_commits_nothing() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("panic");
    touch(&bt, &b).await;

    let doomed = {
        let (bt, b) = (bt.clone(), b.clone());
        tokio::spawn(async move {
            let inner = b.clone();
            let out: AppResult<()> = bt
                .run(&b, move |tx| {
                    Box::pin(async move {
                        write_note(tx, &inner, "must not survive").await?;
                        panic!("deliberate panic inside a BranchTx closure (expected in the log)");
                    })
                })
                .await;
            out
        })
    };
    let joined = within(MUST_FINISH, "the panicking task ends", doomed).await;
    assert!(
        matches!(&joined, Err(e) if e.is_panic()),
        "the panic reaches the caller's task: {joined:?}"
    );

    within(
        Duration::from_secs(5),
        "a run on the branch after a closure panicked — the lock was left behind",
        bt.run(&b, |_tx| Box::pin(async move { Ok(()) })),
    )
    .await
    .expect("the next run commits");

    assert_eq!(
        db.notes(&b).await,
        0,
        "the panicked closure's write is gone"
    );
    assert_eq!(
        version(&bt, &b).await,
        2,
        "only the good runs moved the version"
    );

    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_a_run_midway_releases_the_branch_and_commits_nothing() {
    let db = TestDb::start().await;
    let bt = db.branch_tx();
    let b = branch("dropped");
    touch(&bt, &b).await;

    let (entered_tx, entered_rx) = oneshot::channel::<()>();
    let abandoned = {
        let (bt, b) = (bt.clone(), b.clone());
        tokio::spawn(async move {
            let inner = b.clone();
            let out: AppResult<()> = bt
                .run(&b, move |tx| {
                    Box::pin(async move {
                        write_note(tx, &inner, "must not survive").await?;
                        let _ = entered_tx.send(());
                        std::future::pending::<()>().await;
                        Ok(())
                    })
                })
                .await;
            out
        })
    };
    within(MUST_FINISH, "the run enters its closure", entered_rx)
        .await
        .expect("the run entered its closure");

    // The caller goes away (a client disconnect, a request timeout).
    abandoned.abort();
    let joined = within(MUST_FINISH, "the aborted task ends", abandoned).await;
    assert!(
        matches!(&joined, Err(e) if e.is_cancelled()),
        "the task was cancelled while inside its closure: {joined:?}"
    );

    within(
        Duration::from_secs(5),
        "a run on the branch after another run was dropped midway — the lock was left behind",
        bt.run(&b, |_tx| Box::pin(async move { Ok(()) })),
    )
    .await
    .expect("the next run commits");

    assert_eq!(db.notes(&b).await, 0, "the dropped run's write is gone");
    assert_eq!(
        version(&bt, &b).await,
        2,
        "only the good runs moved the version"
    );

    db.finish().await;
}
