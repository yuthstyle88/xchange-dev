//! Ticket #597 (CM-1), amended by the lane controller's ruling on #691 / #597
//! (option A): a claim whose `run` failed is released, a claim left `PENDING`
//! by a dead process is taken over after a lease, and a late `finish`/`release`
//! of a claim that was taken over changes nothing. Written before the
//! implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §5.6 (the release and lease rule,
//! added with these tests) and §6.
//!
//! Signatures this test assumes (beside those `idempotency_store_test.rs` lists):
//! - `Claim::New(ClaimToken)`: a `New` claim carries the token of that claim.
//!   `ClaimToken` (in `store.rs`) is `Clone + Debug + PartialEq + Eq`; nothing
//!   else about it is pinned.
//! - `finish_in(&self, tx: &mut sqlx::Transaction<'_, Postgres>, op, key, token:
//!   &ClaimToken, response) -> AppResult<()>` writes the answer **in the caller's
//!   transaction** (the command's own, ruling X on #597) and must finish exactly
//!   the record `token` claimed while it is `PENDING`; anything else is an
//!   `Err`, so the caller's transaction (and with it the command's writes)
//!   rolls back. A `finish_in` whose transaction rolls back leaves the claim
//!   `PENDING`.
//! - `release(&self, op, key, token: &ClaimToken) -> AppResult<()>` deletes
//!   only the `PENDING` record **this** token claimed, and is a successful no-op
//!   otherwise (`Ok(())`, nothing changed).
//! - The lease is 60 seconds, measured from `idempotency_records.created_at`,
//!   which a takeover resets to `now()`. A `PENDING` record younger than the
//!   lease is in flight (`Conflict` for the same hash); older, the next claim
//!   with **the same hash** takes it over (`New` with a new token). A different
//!   hash is `Conflict` at any age, and a finished record replays at any age.
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. They fail without it; they do not skip.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Barrier;
use xchange::infrastructure::idempotency::pg_store::PgIdempotencyStore;
use xchange::infrastructure::idempotency::store::{
    Claim, ClaimToken, IdempotencyKey, IdempotencyStore,
};

const MUST_FINISH: Duration = Duration::from_secs(30);

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
                 The idempotency store tests need a real PostgreSQL and do not skip. Point it \
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
        let schema = unique("idemlease");
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
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");
        Self { pool, schema }
    }

    fn store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(PgIdempotencyStore::new(self.pool.clone()))
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

// ------------------------------------------------------------ helpers

fn key(text: &str) -> IdempotencyKey {
    IdempotencyKey::new(text)
}

async fn claim(store: &Arc<dyn IdempotencyStore>, op: &str, k: &str, hash: &str) -> Claim {
    store
        .claim(op, &key(k), hash)
        .await
        .unwrap_or_else(|e| panic!("claim({op}, {k}, {hash}) failed: {e}"))
}

/// `finish_in` in a transaction of its own; committed when it succeeds, rolled
/// back when it fails. Returns whether it succeeded.
async fn try_finish(
    db: &TestDb,
    store: &Arc<dyn IdempotencyStore>,
    op: &str,
    k: &str,
    t: &ClaimToken,
    v: Value,
) -> bool {
    let mut tx = db.pool.begin().await.expect("begin");
    match store.finish_in(&mut tx, op, &key(k), t, v).await {
        Ok(()) => {
            tx.commit().await.expect("commit");
            true
        }
        Err(_) => {
            tx.rollback().await.expect("rollback");
            false
        }
    }
}

async fn finish(
    db: &TestDb,
    store: &Arc<dyn IdempotencyStore>,
    op: &str,
    k: &str,
    t: &ClaimToken,
    v: Value,
) {
    assert!(
        try_finish(db, store, op, k, t, v).await,
        "finish_in({op}, {k}) of the live claim must succeed"
    );
}

async fn release(store: &Arc<dyn IdempotencyStore>, op: &str, k: &str, t: &ClaimToken) {
    store
        .release(op, &key(k), t)
        .await
        .unwrap_or_else(|e| panic!("release({op}, {k}) failed: {e}"));
}

fn is_new(c: &Claim, what: &str) -> ClaimToken {
    match c {
        Claim::New(token) => token.clone(),
        other => panic!("{what}: New, got {other:?}"),
    }
}

fn is_conflict(c: &Claim, what: &str) {
    assert!(matches!(c, Claim::Conflict), "{what}: Conflict, got {c:?}");
}

fn replayed(c: Claim, what: &str) -> Value {
    match c {
        Claim::Replay(v) => v,
        other => panic!("{what}: Replay, got {other:?}"),
    }
}

impl TestDb {
    /// Make the record of `(op, key)` look claimed `seconds` ago.
    async fn age(&self, op: &str, k: &str, seconds: i64) {
        let n = sqlx::query(
            "UPDATE idempotency_records \
             SET created_at = now() - make_interval(secs => $3) \
             WHERE operation = $1 AND key = $2",
        )
        .bind(op)
        .bind(k)
        .bind(seconds as f64)
        .execute(&self.pool)
        .await
        .expect("age the record")
        .rows_affected();
        assert_eq!(n, 1, "exactly one record of ({op}, {k}) to age");
    }

    async fn count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM idempotency_records")
            .fetch_one(&self.pool)
            .await
            .expect("count idempotency_records")
    }
}

// ------------------------------------------------------------ release

/// A failed `run` releases its own claim: the record is gone, and the same key
/// and hash are `New` again (with another token).
#[tokio::test]
async fn release_of_the_own_claim_removes_it_and_the_key_is_new_again() {
    let db = TestDb::start().await;
    let store = db.store();

    let first = is_new(&claim(&store, "op", "k-1", "h-1").await, "first claim");
    release(&store, "op", "k-1", &first).await;
    assert_eq!(db.count().await, 0, "a released claim leaves no record");

    let second = is_new(&claim(&store, "op", "k-1", "h-1").await, "after release");
    assert_ne!(first, second, "a new claim has a new token");
    finish(&db, &store, "op", "k-1", &second, json!({ "run": 2 })).await;
    let c = claim(&store, "op", "k-1", "h-1").await;
    assert_eq!(replayed(c, "after finish"), json!({ "run": 2 }));
    db.finish().await;
}

/// `release` never removes a finished record: a success, once stored, replays.
#[tokio::test]
async fn release_after_finish_changes_nothing() {
    let db = TestDb::start().await;
    let store = db.store();

    let t = is_new(&claim(&store, "op", "k-1", "h-1").await, "claim");
    finish(&db, &store, "op", "k-1", &t, json!("done")).await;
    release(&store, "op", "k-1", &t).await;
    let c = claim(&store, "op", "k-1", "h-1").await;
    assert_eq!(replayed(c, "release after finish"), json!("done"));
    db.finish().await;
}

// ------------------------------------------------------------ the lease

/// Younger than the lease, a `PENDING` record is in flight: `Conflict`.
#[tokio::test]
async fn a_pending_claim_inside_the_lease_is_still_in_flight() {
    let db = TestDb::start().await;
    let store = db.store();

    is_new(&claim(&store, "op", "k-1", "h-1").await, "claim");
    db.age("op", "k-1", 59).await;
    is_conflict(&claim(&store, "op", "k-1", "h-1").await, "59 s old");
    assert_eq!(db.count().await, 1);
    db.finish().await;
}

/// Older than the lease, the next claim with the same hash takes the record
/// over: `New`, with a token other than the dead claimer's, still one record.
#[tokio::test]
async fn a_pending_claim_older_than_the_lease_is_taken_over() {
    let db = TestDb::start().await;
    let store = db.store();

    let dead = is_new(&claim(&store, "op", "k-1", "h-1").await, "claim");
    db.age("op", "k-1", 61).await;
    let taker = is_new(
        &claim(&store, "op", "k-1", "h-1").await,
        "61 s old: taken over",
    );
    assert_ne!(dead, taker, "the takeover has its own token");
    assert_eq!(
        db.count().await,
        1,
        "taken over in place, not a second record"
    );

    // The takeover restarts the lease: a third claimer sees it in flight.
    is_conflict(
        &claim(&store, "op", "k-1", "h-1").await,
        "right after the takeover",
    );
    finish(&db, &store, "op", "k-1", &taker, json!("taker")).await;
    let c = claim(&store, "op", "k-1", "h-1").await;
    assert_eq!(replayed(c, "after the taker finished"), json!("taker"));
    db.finish().await;
}

/// The lease never turns a different payload into a run: another hash is
/// `Conflict` however old the record is, and a finished record replays at any
/// age.
#[tokio::test]
async fn the_lease_never_overrides_the_hash_or_a_finished_answer() {
    let db = TestDb::start().await;
    let store = db.store();

    is_new(&claim(&store, "op", "pending", "h-1").await, "claim");
    db.age("op", "pending", 3600).await;
    is_conflict(
        &claim(&store, "op", "pending", "h-2").await,
        "another hash, an hour old",
    );

    let t = is_new(&claim(&store, "op", "done", "h-1").await, "claim");
    finish(&db, &store, "op", "done", &t, json!("kept")).await;
    db.age("op", "done", 3600).await;
    let c = claim(&store, "op", "done", "h-1").await;
    assert_eq!(replayed(c, "finished an hour ago"), json!("kept"));
    is_conflict(
        &claim(&store, "op", "done", "h-2").await,
        "finished, another hash",
    );
    db.finish().await;
}

// ------------------------------------------------------------ the token

/// [required] After a takeover, the dead claimer's late `finish_in` fails (so
/// its command's transaction rolls back) and changes nothing: the taker's
/// claim stays in flight, and only the taker's answer is ever stored.
#[tokio::test]
async fn a_late_finish_in_by_the_taken_over_claim_fails_and_changes_nothing() {
    let db = TestDb::start().await;
    let store = db.store();

    let dead = is_new(&claim(&store, "op", "k-1", "h-1").await, "claim");
    db.age("op", "k-1", 61).await;
    let taker = is_new(&claim(&store, "op", "k-1", "h-1").await, "takeover");

    assert!(
        !try_finish(&db, &store, "op", "k-1", &dead, json!("dead")).await,
        "the dead claimer's finish_in is an error"
    );
    is_conflict(
        &claim(&store, "op", "k-1", "h-1").await,
        "the dead claimer's finish_in did not finish the taker's claim",
    );

    finish(&db, &store, "op", "k-1", &taker, json!("taker")).await;
    assert!(
        !try_finish(&db, &store, "op", "k-1", &dead, json!("dead again")).await,
        "and is still an error after the taker finished"
    );
    let c = claim(&store, "op", "k-1", "h-1").await;
    assert_eq!(
        replayed(c, "after both finished"),
        json!("taker"),
        "only the taker's answer is stored"
    );
    db.finish().await;
}

/// `finish_in` is part of the caller's transaction: rolled back with it, the
/// claim is still `PENDING` (in flight), never a replay of an answer that was
/// not committed.
#[tokio::test]
async fn a_finish_in_rolled_back_with_its_transaction_leaves_the_claim_pending() {
    let db = TestDb::start().await;
    let store = db.store();

    let t = is_new(&claim(&store, "op", "k-1", "h-1").await, "claim");
    let mut tx = db.pool.begin().await.expect("begin");
    store
        .finish_in(&mut tx, "op", &key("k-1"), &t, json!("never committed"))
        .await
        .expect("finish_in of the live claim");
    tx.rollback().await.expect("rollback");

    is_conflict(
        &claim(&store, "op", "k-1", "h-1").await,
        "rolled back: still in flight, not a replay",
    );
    finish(&db, &store, "op", "k-1", &t, json!("committed")).await;
    let c = claim(&store, "op", "k-1", "h-1").await;
    assert_eq!(
        replayed(c, "after the committed finish"),
        json!("committed")
    );
    db.finish().await;
}

/// `finish_in` of a record that is already finished is an error too: a claim is
/// finished once.
#[tokio::test]
async fn a_second_finish_in_of_a_finished_claim_fails_and_keeps_the_first_answer() {
    let db = TestDb::start().await;
    let store = db.store();

    let t = is_new(&claim(&store, "op", "k-1", "h-1").await, "claim");
    finish(&db, &store, "op", "k-1", &t, json!("first")).await;
    assert!(
        !try_finish(&db, &store, "op", "k-1", &t, json!("second")).await,
        "a second finish_in is an error"
    );
    let c = claim(&store, "op", "k-1", "h-1").await;
    assert_eq!(replayed(c, "after the second finish_in"), json!("first"));
    db.finish().await;
}

/// [required] After a takeover, the dead claimer's late `release` is a no-op:
/// the taker's claim stays, in flight and then finishable.
#[tokio::test]
async fn a_late_release_by_the_taken_over_claim_changes_nothing() {
    let db = TestDb::start().await;
    let store = db.store();

    let dead = is_new(&claim(&store, "op", "k-1", "h-1").await, "claim");
    db.age("op", "k-1", 61).await;
    let taker = is_new(&claim(&store, "op", "k-1", "h-1").await, "takeover");

    release(&store, "op", "k-1", &dead).await;
    assert_eq!(db.count().await, 1, "the taker's record survives");
    is_conflict(
        &claim(&store, "op", "k-1", "h-1").await,
        "still the taker's, in flight",
    );
    finish(&db, &store, "op", "k-1", &taker, json!("taker")).await;
    let c = claim(&store, "op", "k-1", "h-1").await;
    assert_eq!(replayed(c, "taker finished"), json!("taker"));
    db.finish().await;
}

/// Two claimers racing for one expired record: exactly one takes it over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claims_of_an_expired_record_give_exactly_one_new() {
    const ROUNDS: usize = 20;
    const CLAIMERS: usize = 6;
    let db = TestDb::start().await;
    let store = db.store();

    for round in 0..ROUNDS {
        let k = format!("expired-{round}");
        is_new(&claim(&store, "op", &k, "h").await, "the dead claim");
        db.age("op", &k, 61).await;
        let barrier = Arc::new(Barrier::new(CLAIMERS));
        let mut tasks = Vec::new();
        for _ in 0..CLAIMERS {
            let store = store.clone();
            let barrier = barrier.clone();
            let k = k.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                store
                    .claim("op", &IdempotencyKey::new(k.as_str()), "h")
                    .await
            }));
        }
        let mut news = 0;
        for task in tasks {
            let c = task
                .await
                .expect("a claimer task did not panic")
                .unwrap_or_else(|e| panic!("round {round}: a claim failed: {e}"));
            if matches!(c, Claim::New(_)) {
                news += 1;
            }
        }
        assert_eq!(news, 1, "round {round}: exactly one takeover of {CLAIMERS}");
    }
    assert_eq!(db.count().await, ROUNDS as i64);
    db.finish().await;
}
