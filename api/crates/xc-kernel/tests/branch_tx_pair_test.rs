//! Ticket #661 (LG-0) — `BranchTx::run_pair`, one transaction that holds two branches' locks. Written before the
//! implementation by the ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a file of your
//! own.
//!
//! Design: `docs/design/blueprint/ledger-inventory.md` (#553, merged), §2 L-5 and §10 LG-0. A BRANCH transfer's
//! receipt posts to the sender and the receiver in one transaction, so it locks both branches, moves both
//! versions, and must never deadlock against the opposite receipt or against today's `run` + `lock_also`.
//!
//! **The lock order** (the one total order of L-5, kept compatible with `lock_also`'s rule "MAIN first, then the
//! other branch", `branch_tx.rs` doc of `lock_also`): `MAIN` first; otherwise the two ids in ascending byte order.
//!
//! Signatures these tests assume:
//! - `BranchTx::run_pair<T, F>(&self, a: &BranchId, b: &BranchId, f: F) -> AppResult<T>`, where `F` is the same
//!   closure type as `run`'s (`for<'a> FnOnce(&'a mut sqlx::Transaction<'_, Postgres>) -> BoxFuture<'a, AppResult<T>>`).
//! - `run_pair(a, a, f)` behaves exactly as `run(a, f)`: the version rises by one, not two.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`); none skips.
//! Each test has a schema of its own with `api/migrations/` applied. Every wait has a bound.

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::{Barrier, oneshot};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

const MUST_FINISH: Duration = Duration::from_secs(20);
const STAYS_OUT: Duration = Duration::from_millis(400);
const HOLD: Duration = Duration::from_millis(30);

static NEXT: AtomicU64 = AtomicU64::new(0);

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
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. The run_pair tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names '{database}', which does not end in `_test`"
        );
        let schema = unique("lg0");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(16)
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
            .expect("apply api/migrations");
        sqlx::query("CREATE TABLE lg0_notes (branch_id text NOT NULL, note text NOT NULL)")
            .execute(&pool)
            .await
            .expect("create lg0_notes");
        Self { pool, schema }
    }

    fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    async fn version(&self, branch: &BranchId) -> i64 {
        self.branch_tx()
            .version(branch)
            .await
            .expect("read the version")
    }

    async fn version_rows(&self, branch: &BranchId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM branch_versions WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .expect("count branch_versions rows")
    }

    async fn notes(&self, branch: &BranchId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM lg0_notes WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .expect("count notes")
    }

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
    sqlx::query("INSERT INTO lg0_notes (branch_id, note) VALUES ($1, $2)")
        .bind(branch.as_str())
        .bind(note)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// A committed `run_pair` creates both rows if absent and raises both versions by exactly one, whichever argument
/// comes first; what the closure wrote is committed with it.
#[tokio::test(flavor = "multi_thread")]
async fn a_committed_pair_moves_both_versions_by_one() {
    let db = TestDb::start().await;
    let (a, b) = (branch("a"), branch("b"));
    assert_eq!(
        (db.version_rows(&a).await, db.version_rows(&b).await),
        (0, 0)
    );

    let (aa, bb) = (a.clone(), b.clone());
    db.branch_tx()
        .run_pair(&a, &b, move |tx| {
            Box::pin(async move {
                write_note(tx, &aa, "out").await?;
                write_note(tx, &bb, "in").await
            })
        })
        .await
        .expect("the pair commits");
    assert_eq!((db.version(&a).await, db.version(&b).await), (1, 1));
    assert_eq!((db.notes(&a).await, db.notes(&b).await), (1, 1));

    // the other argument order counts the same
    db.branch_tx()
        .run_pair(&b, &a, |_tx| Box::pin(async { Ok(()) }))
        .await
        .expect("the reversed pair commits");
    assert_eq!((db.version(&a).await, db.version(&b).await), (2, 2));
    assert_eq!(
        (db.version_rows(&a).await, db.version_rows(&b).await),
        (1, 1)
    );
    db.finish().await;
}

/// A closure that fails rolls everything back: neither version moves, nothing it wrote is kept, and the error is
/// the closure's own.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_pair_moves_neither_version() {
    let db = TestDb::start().await;
    let (a, b) = (branch("a"), branch("b"));
    let (aa, bb) = (a.clone(), b.clone());
    db.branch_tx()
        .run_pair(&a, &b, |_tx| Box::pin(async { Ok(()) }))
        .await
        .expect("a first pair commits");

    let err = db
        .branch_tx()
        .run_pair(&a, &b, move |tx| {
            Box::pin(async move {
                write_note(tx, &aa, "out").await?;
                write_note(tx, &bb, "in").await?;
                Err::<(), _>(AppError::Domain("refused".to_owned()))
            })
        })
        .await
        .expect_err("the closure refused");
    assert!(
        matches!(&err, AppError::Domain(s) if s == "refused"),
        "the closure's own error comes back: {err:?}"
    );
    assert_eq!((db.version(&a).await, db.version(&b).await), (1, 1));
    assert_eq!((db.notes(&a).await, db.notes(&b).await), (0, 0));
    db.finish().await;
}

/// `run_pair(a, a, f)` is `run(a, f)`: one row, one increment.
#[tokio::test(flavor = "multi_thread")]
async fn a_pair_of_one_branch_is_a_plain_run() {
    let db = TestDb::start().await;
    let a = branch("a");
    db.branch_tx()
        .run_pair(&a, &a, |_tx| Box::pin(async { Ok(()) }))
        .await
        .expect("a pair of one branch commits");
    assert_eq!(db.version(&a).await, 1);
    assert_eq!(db.version_rows(&a).await, 1);
    db.finish().await;
}

/// While a pair holds its two locks, a plain `run` on either branch stays out until the pair commits.
#[tokio::test(flavor = "multi_thread")]
async fn a_held_pair_keeps_both_branches_out() {
    let db = TestDb::start().await;
    let (a, b) = (branch("a"), branch("b"));
    for other in [a.clone(), b.clone()] {
        let (entered_tx, entered_rx) = oneshot::channel::<()>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let bt = db.branch_tx();
        let (pa, pb) = (a.clone(), b.clone());
        let pair = tokio::spawn(async move {
            bt.run_pair(&pa, &pb, move |_tx| {
                Box::pin(async move {
                    let _ = entered_tx.send(());
                    let _ = release_rx.await;
                    Ok(())
                })
            })
            .await
        });
        within(MUST_FINISH, "the pair enters", entered_rx)
            .await
            .expect("the pair signals");

        let bt = db.branch_tx();
        let o = other.clone();
        let mut single =
            tokio::spawn(async move { bt.run(&o, |_tx| Box::pin(async { Ok(()) })).await });
        assert!(
            tokio::time::timeout(STAYS_OUT, &mut single).await.is_err(),
            "a run on {other:?} got in while the pair held its lock"
        );
        let _ = release_tx.send(());
        within(MUST_FINISH, "the pair commits", pair)
            .await
            .expect("join the pair")
            .expect("the pair commits");
        within(MUST_FINISH, "the single run follows", single)
            .await
            .expect("join the run")
            .expect("the run commits");
    }
    db.finish().await;
}

/// Two opposite pairs — a receipt A→B under B and a receipt B→A under A — started together, many times, all
/// finish: one lock order, no cycle.
#[tokio::test(flavor = "multi_thread")]
async fn opposite_pairs_at_once_never_deadlock() {
    let db = TestDb::start().await;
    let (a, b) = (branch("a"), branch("b"));
    const ROUNDS: usize = 25;
    for _ in 0..ROUNDS {
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for (x, y) in [(a.clone(), b.clone()), (b.clone(), a.clone())] {
            let bt = db.branch_tx();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                bt.run_pair(&x, &y, |_tx| {
                    Box::pin(async {
                        tokio::time::sleep(HOLD).await;
                        Ok(())
                    })
                })
                .await
            }));
        }
        for h in handles {
            within(MUST_FINISH, "an opposite pair finishes", h)
                .await
                .expect("join")
                .expect("the pair commits");
        }
    }
    assert_eq!(
        (db.version(&a).await, db.version(&b).await),
        (2 * ROUNDS as i64, 2 * ROUNDS as i64)
    );
    db.finish().await;
}

/// The order is compatible with `lock_also`'s "MAIN first": a `run(MAIN)` that calls `lock_also(x)` and a
/// `run_pair(x, MAIN)` started together, many times, all finish. (`x` sorts before `MAIN`, so an order by id alone
/// would take `x` first and could wait in a cycle with the `run(MAIN)` + `lock_also(x)` transaction.)
#[tokio::test(flavor = "multi_thread")]
async fn a_pair_with_main_takes_main_first_like_lock_also() {
    let db = TestDb::start().await;
    let main = BranchId::new("MAIN".to_owned());
    let x = BranchId::new(unique("A")); // "A…" < "MAIN"
    assert!(x.as_str() < "MAIN");
    const ROUNDS: usize = 25;
    for _ in 0..ROUNDS {
        let barrier = Arc::new(Barrier::new(2));
        let bt1 = db.branch_tx();
        let (m1, x1) = (main.clone(), x.clone());
        let b1 = barrier.clone();
        let via_lock_also = tokio::spawn(async move {
            b1.wait().await;
            bt1.run(&m1, move |tx| {
                Box::pin(async move {
                    tokio::time::sleep(HOLD).await;
                    BranchTx::lock_also(tx, &x1).await
                })
            })
            .await
        });
        let bt2 = db.branch_tx();
        let (m2, x2) = (main.clone(), x.clone());
        let b2 = barrier.clone();
        let via_pair = tokio::spawn(async move {
            b2.wait().await;
            bt2.run_pair(&x2, &m2, |_tx| {
                Box::pin(async {
                    tokio::time::sleep(HOLD).await;
                    Ok(())
                })
            })
            .await
        });
        within(MUST_FINISH, "run + lock_also finishes", via_lock_also)
            .await
            .expect("join")
            .expect("run + lock_also commits");
        within(MUST_FINISH, "run_pair finishes", via_pair)
            .await
            .expect("join")
            .expect("run_pair commits");
    }
    db.finish().await;
}

/// Whether another transaction holds `branch`'s `branch_versions` row (`FOR UPDATE NOWAIT` is refused).
async fn is_locked(pool: &PgPool, branch: &BranchId) -> bool {
    let mut tx = pool.begin().await.expect("begin a probe");
    let probe =
        sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE NOWAIT")
            .bind(branch.as_str())
            .execute(&mut *tx)
            .await;
    let _ = tx.rollback().await;
    probe.is_err()
}

/// The order is decided by the ids, never by the argument order: with the second branch of the order held
/// elsewhere, a pair given its arguments reversed has already taken the first one while it waits; with the first
/// one held elsewhere, it has taken nothing. Checked for two plain ids and for MAIN with an id that sorts before it.
#[tokio::test(flavor = "multi_thread")]
async fn the_pair_locks_in_the_one_order_whatever_the_argument_order() {
    let db = TestDb::start().await;
    let lo = BranchId::new(unique("A"));
    let hi = BranchId::new(unique("B"));
    assert!(lo.as_str() < hi.as_str());
    let main = BranchId::new("MAIN".to_owned());
    let before_main = BranchId::new(unique("A"));
    // (first in the order, second in the order)
    for (first, second) in [
        (lo.clone(), hi.clone()),
        (main.clone(), before_main.clone()),
    ] {
        for b in [&first, &second] {
            db.branch_tx()
                .run(b, |_tx| Box::pin(async { Ok(()) }))
                .await
                .expect("create the row");
        }

        // 1. second held elsewhere ⇒ the pair takes `first`, then waits
        let (held_tx, held_rx) = oneshot::channel::<()>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let bt = db.branch_tx();
        let s = second.clone();
        let holder = tokio::spawn(async move {
            bt.run(&s, move |_tx| {
                Box::pin(async move {
                    let _ = held_tx.send(());
                    let _ = release_rx.await;
                    Ok(())
                })
            })
            .await
        });
        within(MUST_FINISH, "the holder enters", held_rx)
            .await
            .expect("signal");
        let bt = db.branch_tx();
        let (s, f) = (second.clone(), first.clone());
        let pair =
            tokio::spawn(
                async move { bt.run_pair(&s, &f, |_tx| Box::pin(async { Ok(()) })).await },
            );
        within(
            MUST_FINISH,
            "the pair takes the first branch while it waits",
            async {
                while !is_locked(&db.pool, &first).await {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            },
        )
        .await;
        let _ = release_tx.send(());
        within(MUST_FINISH, "holder", holder)
            .await
            .expect("join")
            .expect("commit");
        within(MUST_FINISH, "pair", pair)
            .await
            .expect("join")
            .expect("commit");

        // 2. first held elsewhere ⇒ the pair takes nothing, so `second` stays free
        let (held_tx, held_rx) = oneshot::channel::<()>();
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let bt = db.branch_tx();
        let f = first.clone();
        let holder = tokio::spawn(async move {
            bt.run(&f, move |_tx| {
                Box::pin(async move {
                    let _ = held_tx.send(());
                    let _ = release_rx.await;
                    Ok(())
                })
            })
            .await
        });
        within(MUST_FINISH, "the holder enters", held_rx)
            .await
            .expect("signal");
        let bt = db.branch_tx();
        let (s, f) = (second.clone(), first.clone());
        let pair =
            tokio::spawn(
                async move { bt.run_pair(&s, &f, |_tx| Box::pin(async { Ok(()) })).await },
            );
        let watch_until = tokio::time::Instant::now() + STAYS_OUT;
        while tokio::time::Instant::now() < watch_until {
            assert!(
                !is_locked(&db.pool, &second).await,
                "the pair took {second:?} before {first:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let _ = release_tx.send(());
        within(MUST_FINISH, "holder", holder)
            .await
            .expect("join")
            .expect("commit");
        within(MUST_FINISH, "pair", pair)
            .await
            .expect("join")
            .expect("commit");
    }
    db.finish().await;
}
