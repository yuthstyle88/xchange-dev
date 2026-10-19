//! Ticket #602 (RE-5) — `POST /api/rate-drafts/{id}/simulate` answers the evaluation the A02 screen
//! marks its lines from, and writes nothing.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.5 (a side is out-of-bound when its proposed rate's
//! distance from its reference lies outside the guard), §4.7 (`simulate` is a command with no
//! write: "the diff of AUTO/MANUAL, guard violations, no mutation"), §9 RE-5 (an out-of-bound
//! line is marked), Blueprint §10 (*"ไม่มี mutation published"*).
//!
//! Also here: the evaluation `submit` stores (`rate_drafts.evaluation`, copied into
//! `approval_requests.proposed_payload`, which the A06 queue reads) lists its out-of-bound sides in
//! the same structured shape as simulate, under the key `out_of_bound`. Added in a later commit by
//! the lane controller's ruling 2026-10-09 on #602: it was `["USD:Sell"]`, the Debug text of `Side`
//! (`rate_draft_service.rs:490` at `f04fd32`), and Debug text is not a wire contract. The
//! evaluation hash covers that JSON and changes with it; v3 starts from an empty database.
//!
//! Why this file: a route test that checks only roles lets a stub handler pass. This one pins the
//! answer through the route the screen calls, with the fixture of
//! `crates/xc-rate/tests/rate_draft_flow_test.rs` (`seed_usd`, `out_of_bound_usd`): a BoT USD
//! reference 32.1 / 32.45 and a policy with no adjustment and a USD guard of `[-1, 1]` % on both
//! sides; MANUAL sell 40 is +23 % of 32.45.
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing database whose name
//! ends in `_test`. They fail without it; they do not skip. Each test works in a schema of its own.
//!
//! Signatures, DTO fields and shapes this test assumes (main's at `f04fd32`):
//! - the command seam of `api/src/presentation/http/v3/command.rs`: `Idempotency-Key` required,
//!   the path's `{id}` added to the body, an optional `branch`;
//! - create `POST /api/rate-drafts` `{"lines": [...], "branch"}` answers `{"id", "version",
//!   "status"}` (`commands/rate_draft_save.rs`);
//! - simulate `POST /api/rate-drafts/{id}/simulate` `{"branch"}` answers exactly
//!   `{"id", "isInBound", "outOfBound"}`, `outOfBound` listing each out-of-bound side as an object
//!   `{"currency": "<CODE>", "side": "BUY" | "SELL"}` and nothing else (camelCase keys; the side in
//!   the uppercase wire form the repo already uses for a side, e.g. `transaction_row.rs` `side`).
//!   Re-pinned in a later commit by the lane controller's ruling 2026-10-09 on #602: the first
//!   version of this file pinned `"USD:Sell"`, the `{:?}` (Debug) text of `Side`
//!   (`rate_draft_simulate.rs:81` at `f04fd32`), and Debug text is not a wire contract;
//! - the tables of migration 4 (`rate_reference_batches`, `rate_reference_lines`,
//!   `rate_policy_versions`, `rate_drafts`, `rate_draft_lines`) and AP-1's `approval_requests`,
//!   `approval_decisions`, `audit_events`.

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
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const RATES: &str = "staff:RATES:XC-B01";
const OWNER: &str = "owner";

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
                 The simulate route tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("v3ratesimulate");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01')")
            .execute(&pool)
            .await
            .expect("insert the branch this file names into branches");
        Self { pool, schema }
    }

    async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// A BoT reference batch fetched an hour ago with USD 32.1 / 32.45 per 1 (§4.1, §4.2), and
    /// XC-B01's policy version 1: no adjustment, a USD guard of `[-1, 1]` % on both sides (§4.3).
    /// The same fixture as `rate_draft_flow_test.rs`'s `seed_usd`.
    async fn seed_usd(&self) {
        let batch: String = sqlx::query_scalar(
            "INSERT INTO rate_reference_batches \
               (id, provider, source_date, fetched_at, raw_checksum, status) \
             VALUES (gen_random_uuid(), 'BOT', (now() - interval '1 hour')::date, \
                     now() - interval '1 hour', $1, 'COMPLETE') \
             RETURNING id::text",
        )
        .bind(unique("checksum"))
        .fetch_one(&self.pool)
        .await
        .expect("insert into rate_reference_batches (§4.1)");
        sqlx::query(
            "INSERT INTO rate_reference_lines \
               (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
             VALUES (gen_random_uuid(), $1::uuid, 'USD', 'BOT', 1, 32.1, 32.45, '{}'::jsonb)",
        )
        .bind(&batch)
        .execute(&self.pool)
        .await
        .expect("insert into rate_reference_lines (§4.1)");
        let guard = json!({ "min_pct": "-1.0000", "max_pct": "1.0000" });
        sqlx::query(
            "INSERT INTO rate_policy_versions \
               (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
                max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
                approved_by, reason, source_version) \
             VALUES (gen_random_uuid(), $1, 1, 0, 0, $2, NULL, NULL, NULL, \
                     now() - interval '1 day', NULL, 'owner-1', 'seeded by the test', NULL)",
        )
        .bind(B01)
        .bind(json!({ "USD": { "buy": guard.clone(), "sell": guard } }))
        .execute(&self.pool)
        .await
        .expect("insert into rate_policy_versions (§4.1)");
    }

    /// Everything simulate could write about one draft: its row and its lines, as text.
    async fn draft_snapshot(&self, id: &str) -> (String, String) {
        let row: String = sqlx::query_scalar(
            "SELECT row_to_json(d)::text FROM rate_drafts d WHERE d.id::text = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read draft {id}: {e}"));
        let lines: String = sqlx::query_scalar(
            "SELECT coalesce(string_agg(row_to_json(l)::text, '|' ORDER BY l.id), '') \
             FROM rate_draft_lines l WHERE l.draft_id::text = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read the lines of draft {id}: {e}"));
        (row, lines)
    }

    /// Counts of every table a submit, a decision, a publish or a fetch writes.
    async fn engine_counts(&self) -> Vec<(&'static str, i64)> {
        let mut out = Vec::new();
        for table in [
            "rate_drafts",
            "rate_draft_lines",
            "approval_requests",
            "approval_decisions",
            "audit_events",
            "rate_publications",
            "published_rate_lines",
            "rate_reference_batches",
            "rate_reference_lines",
            "rate_policy_versions",
        ] {
            out.push((table, self.count(table).await));
        }
        out
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

// ------------------------------------------------------------ callers

/// Who is signed in, from the request header `x-test-as`: `owner` or `staff:<ROLE>:<branch>`.
struct HeaderCallers;

fn roles_of(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown role {other}"),
    }
}

fn callers_of(spec: &str) -> Callers {
    if spec == OWNER {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = parts.next().expect("a role");
    let roles = roles_of(role);
    let branch = parts.next().expect("a branch");
    test_callers::callers(
        None,
        Some(test_callers::staff(
            &format!("employee-{role}"),
            roles,
            BranchId::new(branch),
        )),
    )
}

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn command(db: &TestDb, who: &str, method: Method, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// Create a draft of `lines` on XC-B01 as `who`; answers its id.
async fn create_draft(db: &TestDb, who: &str, lines: Vec<Value>) -> String {
    let a = command(
        db,
        who,
        Method::POST,
        "/api/rate-drafts",
        json!({ "branch": B01, "lines": lines }),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "create a draft: {}", a.text);
    assert_eq!(a.body["status"], json!("DRAFT"), "a new draft: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("the draft's id: {}", a.text))
        .to_owned()
}

async fn simulate(db: &TestDb, who: &str, id: &str) -> Answer {
    command(
        db,
        who,
        Method::POST,
        &format!("/api/rate-drafts/{id}/simulate"),
        json!({ "branch": B01 }),
    )
    .await
}

/// USD buy AUTO, sell MANUAL 40 with a reason: +23 % of 32.45, out of the `[-1, 1]` guard.
fn out_of_bound_usd() -> Value {
    json!({
        "currency": "USD",
        "buyMode": "AUTO",
        "sellMode": "MANUAL",
        "manualBuy": null,
        "manualSell": "40",
        "reason": "a spike",
    })
}

/// USD AUTO on both sides: the reference itself at 0 %, inside the guard.
fn auto_usd() -> Value {
    json!({
        "currency": "USD",
        "buyMode": "AUTO",
        "sellMode": "AUTO",
        "manualBuy": null,
        "manualSell": null,
        "reason": null,
    })
}

/// USD MANUAL 32.2 / 32.5 with a reason: +0.31 % and +0.15 % of 32.1 / 32.45, inside the guard.
fn manual_in_bound_usd() -> Value {
    json!({
        "currency": "USD",
        "buyMode": "MANUAL",
        "sellMode": "MANUAL",
        "manualBuy": "32.2",
        "manualSell": "32.5",
        "reason": "rounded by hand",
    })
}

async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn submit(db: &TestDb, who: &str, id: &str) -> Answer {
    command(
        db,
        who,
        Method::POST,
        &format!("/api/rate-drafts/{id}/submit"),
        json!({ "branch": B01 }),
    )
    .await
}

/// `rate_drafts.evaluation -> 'out_of_bound'` and the request's
/// `proposed_payload -> 'out_of_bound'` of one draft.
async fn stored_out_of_bound(db: &TestDb, id: &str) -> (Value, Value) {
    let evaluation: String = sqlx::query_scalar(
        "SELECT (evaluation -> 'out_of_bound')::text FROM rate_drafts WHERE id::text = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("the evaluation of draft {id}: {e}"));
    let payload: String = sqlx::query_scalar(
        "SELECT (proposed_payload -> 'out_of_bound')::text FROM approval_requests \
         WHERE object_id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("the request of draft {id}: {e}"));
    (
        serde_json::from_str(&evaluation).expect("JSON"),
        serde_json::from_str(&payload).expect("JSON"),
    )
}

fn keys_of(body: &Value) -> Vec<String> {
    let mut keys: Vec<String> = body
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}

// ------------------------------------------------------------ tests

#[actix_web::test]
async fn an_out_of_bound_draft_answers_is_in_bound_false_and_lists_the_side_and_writes_nothing() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let id = create_draft(&db, RATES, vec![out_of_bound_usd()]).await;
    let draft_before = db.draft_snapshot(&id).await;
    let counts_before = db.engine_counts().await;

    let a = simulate(&db, RATES, &id).await;
    assert_eq!(a.status, StatusCode::OK, "simulate: {}", a.text);
    assert_eq!(
        keys_of(&a.body),
        vec!["id", "isInBound", "outOfBound"],
        "the answer's keys: {}",
        a.text
    );
    assert_eq!(a.body["id"], json!(id), "the draft simulated: {}", a.text);
    assert_eq!(
        a.body["isInBound"],
        json!(false),
        "MANUAL sell 40 is +23 % of the reference 32.45, outside [-1, 1]: {}",
        a.text
    );
    assert_eq!(
        a.body["outOfBound"],
        json!([{ "currency": "USD", "side": "SELL" }]),
        "exactly the USD sell side, which the screen marks; the AUTO buy side is in bound: {}",
        a.text
    );

    assert_eq!(
        db.draft_snapshot(&id).await,
        draft_before,
        "simulate leaves the draft row and its lines as they were (status, version, evaluation)"
    );
    assert_eq!(
        db.engine_counts().await,
        counts_before,
        "simulate opens no approval request, decides nothing, audits nothing, publishes nothing"
    );

    // the same question twice gives the same answer: nothing moved in between
    let again = simulate(&db, RATES, &id).await;
    assert_eq!(
        again.status,
        StatusCode::OK,
        "simulate again: {}",
        again.text
    );
    assert_eq!(again.body, a.body, "a second simulate answers the same");
    assert_eq!(db.draft_snapshot(&id).await, draft_before);
    db.finish().await;
}

#[actix_web::test]
async fn an_in_bound_draft_answers_is_in_bound_true_with_nothing_listed_and_writes_nothing() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    for (what, who, line) in [
        ("AUTO on both sides", RATES, auto_usd()),
        ("MANUAL inside the guard", OWNER, manual_in_bound_usd()),
    ] {
        let id = create_draft(&db, who, vec![line]).await;
        let draft_before = db.draft_snapshot(&id).await;
        let counts_before = db.engine_counts().await;

        let a = simulate(&db, who, &id).await;
        assert_eq!(a.status, StatusCode::OK, "{what}: simulate: {}", a.text);
        assert_eq!(a.body["id"], json!(id), "{what}: {}", a.text);
        assert_eq!(a.body["isInBound"], json!(true), "{what}: {}", a.text);
        assert_eq!(a.body["outOfBound"], json!([]), "{what}: {}", a.text);

        assert_eq!(
            db.draft_snapshot(&id).await,
            draft_before,
            "{what}: the draft is unchanged"
        );
        assert_eq!(
            db.engine_counts().await,
            counts_before,
            "{what}: nothing was written"
        );
    }
    db.finish().await;
}

#[actix_web::test]
async fn simulate_after_an_edit_answers_for_the_edited_lines() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let id = create_draft(&db, RATES, vec![auto_usd()]).await;
    let first = simulate(&db, RATES, &id).await;
    assert_eq!(first.body["isInBound"], json!(true), "AUTO: {}", first.text);

    let edit = command(
        &db,
        RATES,
        Method::PUT,
        &format!("/api/rate-drafts/{id}"),
        json!({ "branch": B01, "expectedVersion": 1, "lines": [out_of_bound_usd()] }),
    )
    .await;
    assert_eq!(edit.status, StatusCode::OK, "edit: {}", edit.text);

    let second = simulate(&db, RATES, &id).await;
    assert_eq!(second.status, StatusCode::OK, "simulate: {}", second.text);
    assert_eq!(
        second.body["isInBound"],
        json!(false),
        "the edited line is out of bound: {}",
        second.text
    );
    assert_eq!(
        second.body["outOfBound"],
        json!([{ "currency": "USD", "side": "SELL" }]),
        "{}",
        second.text
    );
    assert_eq!(db.count("approval_requests").await, 0, "still no request");
    db.finish().await;
}

#[actix_web::test]
async fn an_out_of_bound_submit_stores_the_structured_list_the_approval_queue_reads() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let id = create_draft(&db, RATES, vec![out_of_bound_usd()]).await;
    let simulated = simulate(&db, RATES, &id).await;
    assert_eq!(
        simulated.status,
        StatusCode::OK,
        "simulate: {}",
        simulated.text
    );

    let a = submit(&db, RATES, &id).await;
    assert_eq!(a.status, StatusCode::OK, "submit: {}", a.text);
    assert_eq!(
        a.body["status"],
        json!("PENDING_APPROVAL"),
        "out of bound by RATES waits for OWNER: {}",
        a.text
    );

    let expected = json!([{ "currency": "USD", "side": "SELL" }]);
    let (evaluation, payload) = stored_out_of_bound(&db, &id).await;
    assert_eq!(
        evaluation, expected,
        "rate_drafts.evaluation lists the side as {{currency, side}}"
    );
    assert_eq!(
        payload, expected,
        "the approval request's proposed_payload lists the same"
    );
    assert_eq!(
        simulated.body["outOfBound"], expected,
        "simulate and the stored evaluation list the same sides in the same shape"
    );

    let q = get(
        &db,
        OWNER,
        &format!("/api/v3/views/approval_requests?branch={B01}&status=PENDING"),
    )
    .await;
    assert_eq!(q.status, StatusCode::OK, "the A06 queue: {}", q.text);
    let requests = q.body["data"]["requests"]
        .as_array()
        .unwrap_or_else(|| panic!("requests: {}", q.text))
        .clone();
    let mine: Vec<&Value> = requests
        .iter()
        .filter(|r| r["objectId"] == json!(id))
        .collect();
    assert_eq!(mine.len(), 1, "one request for the draft: {}", q.text);
    assert_eq!(
        mine[0]["proposedPayload"]["out_of_bound"], expected,
        "what A06 reads: {}",
        q.text
    );
    db.finish().await;
}

#[actix_web::test]
async fn an_in_bound_submit_stores_an_empty_list() {
    let db = TestDb::start().await;
    db.seed_usd().await;
    let id = create_draft(&db, RATES, vec![auto_usd()]).await;
    let a = submit(&db, RATES, &id).await;
    assert_eq!(a.status, StatusCode::OK, "submit: {}", a.text);
    assert_eq!(a.body["status"], json!("APPROVED"), "{}", a.text);
    let (evaluation, payload) = stored_out_of_bound(&db, &id).await;
    assert_eq!(evaluation, json!([]));
    assert_eq!(payload, json!([]));
    db.finish().await;
}
