//! Ticket #704 Part 2 (CM-3): an idempotency record belongs to a real organisation. `organization_id` (OR-1)
//! is the record's only organisation column and part of its primary key; `org_id` and its placeholder
//! constant are gone; `CommandEndpoint` passes the organisation of the policy-checked branch. Written before
//! the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §5.6 ("until #550 adds organisations, `org_id` is the single
//! organisation's constant"); OR-1 D-1 (`org-roles-branches.md`); ticket #704 Part 2 (data migration
//! `api/migrations/20261030160000_idempotency_organization.sql`).
//!
//! Signatures this test assumes:
//! - `PgIdempotencyStore::new(pool: PgPool, organization_id: impl Into<String>)` (the organisation's id as
//!   `Organization::id` holds it, a uuid in text); there is no constructor without an organisation.
//! - `CommandEndpoint` reads `branches.organization_id` of the policy-checked branch before `claim` and keys
//!   the record by it. A `Locks::Pair` whose second branch belongs to another organisation is refused `403`
//!   `outOfScope` before any record or write.
//! - After every migration: `idempotency_records` has no `org_id` column, and its primary key is
//!   `(organization_id, operation, key)`.
//! - The repo has no down migrations (`api/migrations/*.down.sql` is empty), so nothing about `down` is pinned.
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing database whose name ends in
//! `_test`. They fail without it; they do not skip.

mod common;

use std::borrow::Cow;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use ts_rs::TS;
use xc_auth::domain::access::{Actor as AccessActor, Role as AccessRole, ScopedRole};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::idempotency::pg_store::PgIdempotencyStore;
use xchange::infrastructure::idempotency::store::{Claim, IdempotencyKey, IdempotencyStore};
use xchange::presentation::http::v3::command::{CommandAction, CommandEndpoint, Locks};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
/// OR-1's install organisation (`20261011100000_organizations.sql`).
const XC: &str = "00000000-0000-0000-0000-000000000001";
/// CM-3's migration; the migration test stops just before it.
const CM3_MIGRATION: i64 = 20261030160000;

async fn migrate(pool: &PgPool, last: Option<i64>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut migrator = Migrator::new(dir.as_path())
        .await
        .expect("read api/migrations");
    if let Some(last) = last {
        let kept: Vec<_> = migrator
            .migrations
            .iter()
            .filter(|m| m.version <= last)
            .cloned()
            .collect();
        migrator.migrations = Cow::Owned(kept);
    }
    migrator
        .run(pool)
        .await
        .unwrap_or_else(|e| panic!("apply api/migrations through {last:?}: {e}"));
}

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
        Self::start_through(None).await
    }

    /// Every migration, or only those up to `last`; then this file's branches and table.
    async fn start_through(last: Option<i64>) -> Self {
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
        let schema = unique("idemorg");
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
        migrate(&pool, last).await;
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

impl TestDb {
    /// A second organisation with one branch of its own; returns `(organization id, branch id)`.
    async fn other_org(&self, code: &str) -> (String, String) {
        let org: String = sqlx::query_scalar(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES (gen_random_uuid(), $1, NULL, 'Asia/Bangkok', now()) RETURNING id::text",
        )
        .bind(code)
        .fetch_one(&self.pool)
        .await
        .expect("insert an organisation");
        let branch = format!("{code}-B1");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&branch)
            .bind(&org)
            .execute(&self.pool)
            .await
            .expect("insert its branch");
        (org, branch)
    }

    /// `(organization_id, operation, key)` of every record.
    async fn keys(&self) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "SELECT organization_id::text, operation, key FROM idempotency_records \
             ORDER BY organization_id, operation, key",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read idempotency_records")
    }

    async fn has_column(&self, column: &str) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'idempotency_records' \
               AND column_name = $1",
        )
        .bind(column)
        .fetch_one(&self.pool)
        .await
        .expect("read information_schema")
            == 1
    }

    /// The primary key's columns, in key order.
    async fn primary_key(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT a.attname::text FROM pg_index i \
             JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
             WHERE i.indrelid = 'idempotency_records'::regclass AND i.indisprimary \
             ORDER BY array_position(i.indkey, a.attnum)",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read the primary key")
    }

    fn store(&self, organization: &str) -> Arc<dyn IdempotencyStore> {
        Arc::new(PgIdempotencyStore::new(self.pool.clone(), organization))
    }
}

fn key(text: &str) -> IdempotencyKey {
    IdempotencyKey::new(text)
}

async fn finish(db: &TestDb, store: &Arc<dyn IdempotencyStore>, token_claim: Claim, v: Value) {
    let Claim::New(token) = token_claim else {
        panic!("expected New, got {token_claim:?}")
    };
    let mut tx = db.pool.begin().await.expect("begin");
    store
        .finish_in(&mut tx, "op", &key("k-1"), &token, v)
        .await
        .expect("finish_in");
    tx.commit().await.expect("commit");
}

fn replayed(c: Claim, what: &str) -> Value {
    match c {
        Claim::Replay(v) => v,
        other => panic!("{what}: Replay, got {other:?}"),
    }
}

// ------------------------------------------------------------ the store

/// The schema after every migration: one organisation column, in the key.
#[tokio::test]
async fn the_record_is_keyed_by_organization_operation_key_and_has_no_org_id() {
    let db = TestDb::start().await;
    assert!(!db.has_column("org_id").await, "org_id is gone");
    assert!(db.has_column("organization_id").await);
    assert_eq!(
        db.primary_key().await,
        vec!["organization_id", "operation", "key"]
    );
    db.finish().await;
}

/// A store writes its own organisation, and one key under two organisations is two records, each
/// replaying only its own answer.
#[tokio::test]
async fn one_key_under_two_organisations_is_two_records() {
    let db = TestDb::start().await;
    let (o2, _) = db.other_org("O2").await;
    let (xc, other) = (db.store(XC), db.store(&o2));

    let a = xc.claim("op", &key("k-1"), "h").await.expect("claim XC");
    let b = other.claim("op", &key("k-1"), "h").await.expect("claim O2");
    assert!(matches!(a, Claim::New(_)), "XC: New, got {a:?}");
    assert!(
        matches!(b, Claim::New(_)),
        "O2: New, not a conflict with XC's, got {b:?}"
    );
    finish(&db, &xc, a, json!("xc")).await;
    finish(&db, &other, b, json!("o2")).await;

    let c = xc.claim("op", &key("k-1"), "h").await.expect("replay XC");
    assert_eq!(replayed(c, "XC"), json!("xc"));
    let c = other
        .claim("op", &key("k-1"), "h")
        .await
        .expect("replay O2");
    assert_eq!(replayed(c, "O2"), json!("o2"));

    let mut want = vec![
        (XC.to_owned(), "op".to_owned(), "k-1".to_owned()),
        (o2.clone(), "op".to_owned(), "k-1".to_owned()),
    ];
    want.sort();
    assert_eq!(db.keys().await, want);
    db.finish().await;
}

// ------------------------------------------------------------ the migration

/// A record written before CM-3's migration (with the old placeholder `org_id`) keeps replaying under
/// `(XC, operation, key)` after it; the migration backfills nothing because OR-1 already set
/// `organization_id = XC` on every row.
#[tokio::test]
async fn a_record_from_before_the_migration_replays_under_the_install_organisation() {
    let db = TestDb::start_through(Some(CM3_MIGRATION - 1)).await;
    assert!(
        db.has_column("org_id").await,
        "before CM-3, CM-1's org_id is there"
    );
    sqlx::query(
        "INSERT INTO idempotency_records \
           (org_id, operation, key, request_hash, response_ref, status, created_at, claim_token) \
         VALUES ('00000000-0000-4000-8000-000000000000', 'op', 'k-old', 'h-old', '\"kept\"'::jsonb, \
                 'FINISHED', now(), 'token-old')",
    )
    .execute(&db.pool)
    .await
    .expect("insert a record the CM-1 way");

    migrate(&db.pool, None).await;

    assert!(!db.has_column("org_id").await, "the migration drops org_id");
    let c = db
        .store(XC)
        .claim("op", &key("k-old"), "h-old")
        .await
        .expect("claim after the migration");
    assert_eq!(replayed(c, "the old record"), json!("kept"));
    db.finish().await;
}

// ------------------------------------------------------------ the endpoint

struct HeaderSessions;

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let branch = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        let pool = req
            .app_data::<web::Data<AppState>>()
            .map(|state| state.db.clone());
        Box::pin(async move {
            let org = match pool {
                Some(pool) => sqlx::query_scalar::<_, String>(
                    "SELECT organization_id::text FROM branches WHERE id = $1",
                )
                .bind(&branch)
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| XC.to_owned()),
                None => XC.to_owned(),
            };
            let staff = (!branch.is_empty()).then(|| {
                AccessActor::new(
                    "employee-7",
                    &org,
                    vec![ScopedRole::new(
                        AccessRole::Teller,
                        Some(BranchId::new(branch)),
                        chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                        None,
                    )],
                    None,
                )
            });
            test_callers::callers(None, staff)
        })
    }
}

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct MarkInput {
    #[serde(default)]
    second: Option<String>,
    tag: String,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct Marked {
    branch: String,
    tag: String,
}

/// One branch by default; `Pair(branch, second)` when `second` is given. Writes one mark per branch.
struct Mark;

impl CommandAction for Mark {
    const NAME: &'static str = "test-mark";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "adminSnapshot");
    type Input = MarkInput;
    type Output = Marked;
    const ROUTE: &'static str = "/api/test-mark";
    const METHOD: Method = Method::POST;

    fn locks(&self, branch: &BranchId, input: &MarkInput) -> Locks {
        match &input.second {
            Some(second) => Locks::Pair(branch.clone(), BranchId::new(second.as_str())),
            None => Locks::One(branch.clone()),
        }
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        _who: &AccessActor,
        input: MarkInput,
        _key: IdempotencyKey,
    ) -> Result<Marked, V3Error> {
        sqlx::query("INSERT INTO test_marks (branch_id, tag) VALUES ($1, $2)")
            .bind(branch.as_str())
            .bind(input.tag.as_str())
            .execute(&mut **tx)
            .await
            .expect("mark through run's tx");
        Ok(Marked {
            branch: branch.as_str().to_owned(),
            tag: input.tag,
        })
    }
}

struct Answer {
    status: StatusCode,
    body: Value,
    text: String,
}

async fn post(db: &TestDb, at: &str, body: Value, key: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                CommandEndpoint::register(cfg, Arc::new(Mark));
                bootstrap::configure(cfg, state);
            })
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri("/api/test-mark")
        .insert_header((header::HOST, HOST))
        .insert_header((AS, at))
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

/// The endpoint keys the record by the organisation of the caller's branch: the same key used at a branch of
/// XC and at a branch of another organisation is two records, each its own answer.
#[actix_web::test]
async fn a_command_keys_its_record_by_the_organisation_of_its_branch() {
    let db = TestDb::start().await;
    let (o2, o2_branch) = db.other_org("O2").await;

    let a = post(&db, "XC-B02", json!({ "tag": "at-xc" }), "same-key").await;
    assert_eq!(a.status, StatusCode::OK, "XC; body {}", a.text);
    let b = post(&db, &o2_branch, json!({ "tag": "at-o2" }), "same-key").await;
    assert_eq!(
        b.status,
        StatusCode::OK,
        "O2: not a conflict with XC's key; body {}",
        b.text
    );
    assert_eq!(
        b.body["tag"],
        json!("at-o2"),
        "O2 ran its own command, not a replay of XC's"
    );

    let mut want = vec![
        (XC.to_owned(), "test-mark".to_owned(), "same-key".to_owned()),
        (o2, "test-mark".to_owned(), "same-key".to_owned()),
    ];
    want.sort();
    assert_eq!(db.keys().await, want);
    db.finish().await;
}

/// A pair across two organisations is refused before anything is written.
#[actix_web::test]
async fn a_pair_across_organisations_is_out_of_scope() {
    let db = TestDb::start().await;
    let (_, o2_branch) = db.other_org("O2").await;
    let a = post(
        &db,
        "XC-B02",
        json!({ "second": o2_branch, "tag": "across" }),
        "k-across",
    )
    .await;
    assert_eq!(a.status.as_u16(), 403, "body {}", a.text);
    assert_eq!(
        a.body["error"]["code"],
        json!("outOfScope"),
        "body {}",
        a.text
    );
    assert!(db.keys().await.is_empty(), "no record");
    let marks: i64 = sqlx::query_scalar("SELECT count(*) FROM test_marks")
        .fetch_one(&db.pool)
        .await
        .expect("count marks");
    assert_eq!(marks, 0, "nothing written");
    db.finish().await;
}
