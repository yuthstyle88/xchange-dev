//! Ticket #597 (CM-1) — the idempotency store every v3 command shares: the port
//! `IdempotencyStore` and `PgIdempotencyStore` over `idempotency_records`.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` (#547, #548, #549), §6 "The command seam"
//! (CM-1 owns the table, the port and `PgIdempotencyStore`), §5.1
//! `idempotency_records(org_id, operation, key, request_hash, response_ref,
//! status, created_at, PRIMARY KEY (org_id, operation, key))`, §5.6 (the same key
//! with the same hash replays, with a different hash is refused), §8 migration 3
//! (`api/migrations/20261010115000_idempotency.sql`).
//!
//! Signatures this test assumes (choices where §6 leaves the exact form open):
//! - `xchange::infrastructure::idempotency::store::{IdempotencyStore, Claim,
//!   IdempotencyKey}` (file `api/src/infrastructure/idempotency/store.rs`) and
//!   `xchange::infrastructure::idempotency::pg_store::PgIdempotencyStore` (file
//!   `.../pg_store.rs`, §7).
//! - `IdempotencyStore: Send + Sync` is dyn-safe like the repo's other ports
//!   (`BoxFuture<'a, AppResult<_>>`, `Send` futures), so it is held here as
//!   `Arc<dyn IdempotencyStore>`:
//!   - `claim(&self, op: &str, key: &IdempotencyKey, hash: &str) -> AppResult<Claim>`;
//!   - `finish_in(&self, tx: &mut sqlx::Transaction<'_, Postgres>, op: &str, key: &IdempotencyKey, token: &ClaimToken, response: serde_json::Value) -> AppResult<()>`:
//!     it writes the answer in the caller's transaction (the command's own), and
//!     is an error unless it finished exactly the record `token` claimed (#691,
//!     the lane controller's ruling X on #597).
//! - `Claim` is `Debug` with the variants `New(ClaimToken)`, `Replay(serde_json::Value)` and
//!   `Conflict`. `ClaimToken` binds `finish`/`release` to the claim that won (#691,
//!   lane controller's ruling on #597; its own tests are `idempotency_lease_test.rs`). What a claim of a key that is claimed but not yet finished
//!   answers with the same hash is not pinned here, except that it is never `New`.
//! - `IdempotencyKey::new(&str)` builds a key (an `impl Into<String>` parameter
//!   also satisfies this).
//! - `PgIdempotencyStore::new(pool: PgPool, organization_id: impl Into<String>)`: the store
//!   writes and reads only that organisation's records (#704 Part 2, CM-3: `organization_id`
//!   is the record's one organisation column; OR-1's install organisation `XC` is
//!   `00000000-0000-0000-0000-000000000001`). Retired by the ticket author: the
//!   one-argument `new` and the `org_id` column.
//! - `request_hash` stores the `hash` argument as given; `response_ref` stores
//!   the finished response as given.
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. They fail without it; they do not skip.
//! Each test works in a schema of its own.

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

/// OR-1's install organisation (`20261011100000_organizations.sql`).
const XC: &str = "00000000-0000-0000-0000-000000000001";

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
        let schema = unique("idemstore");
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
        Arc::new(PgIdempotencyStore::new(self.pool.clone(), XC))
    }

    /// Every row as `(organization_id, operation, key, request_hash, response_ref)`.
    /// `response_ref` is read as text (the crate has no sqlx `json` feature).
    async fn rows(&self) -> Vec<(String, String, String, String, Option<Value>)> {
        sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
            "SELECT organization_id::text, operation, key, request_hash, response_ref::text \
             FROM idempotency_records ORDER BY operation, key",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read idempotency_records (migration 3 creates it)")
        .into_iter()
        .map(|(org, op, k, hash, response)| {
            let response =
                response.map(|t| serde_json::from_str(&t).expect("response_ref is JSON"));
            (org, op, k, hash, response)
        })
        .collect()
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
        .unwrap_or_else(|e| panic!("claim({op}, {k}, {hash}) failed: {e:?}"))
}

/// `finish_in` in a transaction of its own, committed.
async fn finish(
    db: &TestDb,
    store: &Arc<dyn IdempotencyStore>,
    op: &str,
    k: &str,
    token: &ClaimToken,
    response: Value,
) {
    let mut tx = db.pool.begin().await.expect("begin");
    store
        .finish_in(&mut tx, op, &key(k), token, response)
        .await
        .unwrap_or_else(|e| panic!("finish_in({op}, {k}) failed: {e:?}"));
    tx.commit().await.expect("commit");
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

// ------------------------------------------------------------ the cycle

/// A new key is `New`; once finished, the same key and hash are `Replay` of
/// exactly the finished response, every time.
#[tokio::test]
async fn a_new_key_is_new_then_finished_then_replayed() {
    let db = TestDb::start().await;
    let store = db.store();
    let answer = json!({ "draftId": "d-1", "status": "SUBMITTED", "lines": [1, 2, { "x": null }] });

    let token = is_new(
        &claim(&store, "rateDraftSubmit", "k-1", "h-1").await,
        "first claim",
    );
    finish(
        &db,
        &store,
        "rateDraftSubmit",
        "k-1",
        &token,
        answer.clone(),
    )
    .await;

    for n in 1..=3 {
        let c = claim(&store, "rateDraftSubmit", "k-1", "h-1").await;
        assert_eq!(replayed(c, &format!("claim {n} after finish")), answer);
    }

    let rows = db.rows().await;
    assert_eq!(rows.len(), 1, "one row: {rows:?}");
    let (organization, operation, k, hash, response) = rows.into_iter().next().expect("one row");
    assert_eq!(
        organization, XC,
        "the record belongs to the store's organisation"
    );
    assert_eq!(
        (operation.as_str(), k.as_str(), hash.as_str()),
        ("rateDraftSubmit", "k-1", "h-1"),
        "the row is keyed (organization_id, operation, key) and keeps the hash"
    );
    assert_eq!(
        response,
        Some(answer),
        "response_ref holds the finished response"
    );
    db.finish().await;
}

/// §5.6: the same key with a different hash is `Conflict` — before and after the
/// first claim finished — and a conflict changes nothing that is stored.
#[tokio::test]
async fn the_same_key_with_a_different_hash_is_a_conflict() {
    let db = TestDb::start().await;
    let store = db.store();
    let answer = json!({ "ok": true });

    let token = is_new(&claim(&store, "op-a", "k-1", "h-1").await, "first claim");
    is_conflict(
        &claim(&store, "op-a", "k-1", "h-2").await,
        "another hash while the first is in flight",
    );
    finish(&db, &store, "op-a", "k-1", &token, answer.clone()).await;
    for hash in ["h-2", "h-3", "H-1"] {
        is_conflict(
            &claim(&store, "op-a", "k-1", hash).await,
            &format!("hash {hash:?} after finish"),
        );
    }

    let c = claim(&store, "op-a", "k-1", "h-1").await;
    assert_eq!(
        replayed(c, "the first hash after the conflicts"),
        answer,
        "the conflicts overwrote nothing"
    );
    let rows = db.rows().await;
    assert_eq!(rows.len(), 1, "still one row: {rows:?}");
    assert_eq!(rows[0].3, "h-1", "the stored hash is the first one");
    db.finish().await;
}

/// §6: the key is `(org_id, operation, key)` — one key under two operations is
/// two independent records.
#[tokio::test]
async fn one_key_under_two_operations_is_two_records() {
    let db = TestDb::start().await;
    let store = db.store();

    let ta = is_new(&claim(&store, "op-a", "shared", "h-a").await, "op-a");
    let tb = is_new(
        &claim(&store, "op-b", "shared", "h-b").await,
        "op-b, same key, another hash: not a conflict",
    );
    let tc = is_new(
        &claim(&store, "op-c", "shared", "h-a").await,
        "op-c, same key, same hash as op-a: not a replay",
    );
    finish(&db, &store, "op-a", "shared", &ta, json!("a")).await;
    finish(&db, &store, "op-b", "shared", &tb, json!("b")).await;
    finish(&db, &store, "op-c", "shared", &tc, json!("c")).await;

    let c = claim(&store, "op-a", "shared", "h-a").await;
    assert_eq!(replayed(c, "op-a"), json!("a"));
    let c = claim(&store, "op-b", "shared", "h-b").await;
    assert_eq!(replayed(c, "op-b"), json!("b"));
    let c = claim(&store, "op-c", "shared", "h-a").await;
    assert_eq!(replayed(c, "op-c"), json!("c"));
    is_conflict(
        &claim(&store, "op-b", "shared", "h-a").await,
        "op-b, op-a's hash",
    );

    let ops: Vec<(String, String)> = db
        .rows()
        .await
        .into_iter()
        .map(|(_, op, k, _, _)| (op, k))
        .collect();
    assert_eq!(
        ops,
        vec![
            ("op-a".to_string(), "shared".to_string()),
            ("op-b".to_string(), "shared".to_string()),
            ("op-c".to_string(), "shared".to_string()),
        ]
    );
    db.finish().await;
}

// ------------------------------------------------------------ concurrency

/// §09: two (or more) concurrent claims of one new key give exactly one `New`;
/// every other claimer is told something else, so only one caller runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claims_of_a_new_key_give_exactly_one_new() {
    const ROUNDS: usize = 25;
    const CLAIMERS: usize = 8;
    let db = TestDb::start().await;
    let store = db.store();

    for round in 0..ROUNDS {
        let k = format!("race-{round}");
        let barrier = Arc::new(Barrier::new(CLAIMERS));
        let mut tasks = Vec::new();
        for _ in 0..CLAIMERS {
            let store = store.clone();
            let barrier = barrier.clone();
            let k = k.clone();
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                store
                    .claim("op-race", &IdempotencyKey::new(k.as_str()), "h-race")
                    .await
            }));
        }
        let mut news = 0;
        for task in tasks {
            let c = task
                .await
                .expect("a claimer task did not panic")
                .unwrap_or_else(|e| panic!("round {round}: a concurrent claim failed: {e:?}"));
            if matches!(c, Claim::New(_)) {
                news += 1;
            }
        }
        assert_eq!(news, 1, "round {round}: exactly one New of {CLAIMERS}");
    }

    // Two claimers is the case §6 names: run it on its own too.
    for round in 0..ROUNDS {
        let k = format!("pair-{round}");
        let k1 = IdempotencyKey::new(k.as_str());
        let k2 = IdempotencyKey::new(k.as_str());
        let (a, b) = tokio::join!(
            store.claim("op-pair", &k1, "h-pair"),
            store.claim("op-pair", &k2, "h-pair"),
        );
        let a = a.expect("claim a");
        let b = b.expect("claim b");
        let news = [&a, &b]
            .iter()
            .filter(|c| matches!(c, Claim::New(_)))
            .count();
        assert_eq!(
            news, 1,
            "pair {round}: exactly one New, got {a:?} and {b:?}"
        );
    }

    let rows = db.rows().await;
    assert_eq!(rows.len(), 2 * ROUNDS, "one row per key: {}", rows.len());
    db.finish().await;
}
