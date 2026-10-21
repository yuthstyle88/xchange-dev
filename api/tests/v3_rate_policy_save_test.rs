//! Ticket #602 (RE-5) — `POST /api/rate-policies` (`ratePolicySave`) for OWNER and RATES: RATES may
//! move only the two percents, every save is a new immutable version chained to the one it
//! replaces.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! The rule (lane controller's ruling 2026-10-09: RATE_MANAGER may save a policy version changing
//! only buy/sell %, per Blueprint RATE_MANAGER "ปรับ %"):
//! - Blueprint §02: RATE_MANAGER *"ร่างเรท ปรับ % เสนอเผยแพร่ | การเปลี่ยนนอกกรอบต้องอนุมัติ"*;
//!   §07 RatePolicyVersion *"immutable หลังอนุมัติ; เหตุผลและ source version"*.
//! - `ratePolicySave` is allowed for OWNER and RATES on Rpc (until #602 it was OWNER only,
//!   `xc-auth/src/domain/policy/entities/policy.rs:70`).
//! - RATES may change only `buyAdjustPct` / `sellAdjustPct`. The guards, `maxReferenceAgeHours`,
//!   `onStale` and `staleLimitThb` must equal the branch's current policy version's: the guards are
//!   what make an out-of-bound draft need OWNER at submit (`docs/design/blueprint/rates.md` §4.5,
//!   §4.6), so RATES must not move them. A RATES save that changes any of them is refused and writes
//!   nothing: no new version, no `valid_to` moved.
//! - It follows that RATES cannot save on a branch with no policy version yet: there is no frame to
//!   keep, and the first frame is OWNER's (A3: "OWNER enters them in the policy screen").
//! - OWNER may change everything.
//! - Every save inserts a new version: `version` = the branch's last + 1, `approved_by` = the
//!   saver's id, `reason` required and non-empty, `source_version` = the version it replaces (NULL
//!   for a branch's first), and the replaced version gets its `valid_to`; exactly one version of a
//!   branch is current (`valid_to IS NULL`).
//!
//! Choices made here (the implementer follows them):
//! - The refusal of a RATES save that moves the frame is the code the policy answers when a staff
//!   role on Rpc is outside a row, `PolicyError::RoleNotAllowedHere` → `roleNotAllowedHere`, with
//!   its status (`policy_error.rs` `status()`, 400). The test compares against `PolicyError` itself.
//! - "Equal" means: `guards` equal as JSON values, the other three equal as values (`staleLimitThb`
//!   as a decimal). The tests send back exactly what the `rate_policy` view answered, as the A02
//!   screen does.
//! - A blank `reason` (empty or only spaces) is `validationFailed` with field `reason`, 400.
//! - A staff session always runs on its own branch (`Policy::branch`, `policy.rs:43-53`): a RATES
//!   member of another branch who names this branch reaches only its own, so it cannot save this
//!   branch's policy.
//!
//! Signatures, DTO fields and shapes this test assumes (main's at `f04fd32`):
//! `commands/rate_policy_save.rs` `RatePolicySaveInput` / `RatePolicySaveOutput` (`{id, version}`),
//! `views/rate_policy.rs` (`{policy: RatePolicyVersionDto | null}`), the command seam of
//! `http/v3/command.rs`, and the table `rate_policy_versions` of migration 4
//! (`20261010120000_rate_engine.sql:26-42`, with `source_version`).
//!
//! The tests need a real PostgreSQL: `TEST_DATABASE_URL` must name an existing database whose name
//! ends in `_test`. They fail without it; they do not skip. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::Role;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const OWNER: &str = "owner";
const OWNER_ID: &str = "owner-1";
const RATES: &str = "staff:RATES:XC-B01";
const RATES_ID: &str = "employee-RATES";
const RATES_OF_B09: &str = "staff:RATES:XC-B09";

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
                 The policy save route tests need a real PostgreSQL and do not skip. Point it at \
                 an existing database whose name ends in `_test`, e.g.\n\
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
        let schema = unique("v3ratepolicysave");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .execute(&pool)
            .await
            .expect("insert the branches this file names into branches");
        Self { pool, schema }
    }

    /// Every policy version of `branch`, oldest first, each as its row's JSON text.
    async fn versions_of(&self, branch: &str) -> Vec<Value> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT row_to_json(p)::text FROM rate_policy_versions p \
             WHERE p.branch_id = $1 ORDER BY p.version",
        )
        .bind(branch)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read the policy versions of {branch}: {e}"));
        rows.iter()
            .map(|r| serde_json::from_str(r).expect("row_to_json is JSON"))
            .collect()
    }

    /// The version `n` of `branch`, with the numeric columns as their database text.
    async fn version(&self, branch: &str, n: i32) -> Value {
        let row: String = sqlx::query_scalar(
            "SELECT json_build_object(\
               'version', version, \
               'buy_adjust_pct', buy_adjust_pct::text, \
               'sell_adjust_pct', sell_adjust_pct::text, \
               'guards', guards, \
               'max_reference_age_hours', max_reference_age_hours, \
               'on_stale', on_stale, \
               'stale_limit_thb', stale_limit_thb::text, \
               'approved_by', approved_by, \
               'reason', reason, \
               'source_version', source_version, \
               'valid_from_set', valid_from IS NOT NULL, \
               'current', valid_to IS NULL)::text \
             FROM rate_policy_versions WHERE branch_id = $1 AND version = $2",
        )
        .bind(branch)
        .bind(n)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read version {n} of {branch}: {e}"));
        serde_json::from_str(&row).expect("json_build_object is JSON")
    }

    async fn current_count(&self, branch: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM rate_policy_versions WHERE branch_id = $1 AND valid_to IS NULL",
        )
        .bind(branch)
        .fetch_one(&self.pool)
        .await
        .expect("count the current policy versions")
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
        return test_callers::callers(Some(test_callers::owner(OWNER_ID)), None);
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

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// `POST /api/rate-policies` as `who`, naming `branch`.
async fn save(db: &TestDb, who: &str, branch: &str, body: Value) -> Answer {
    let mut body = body;
    body["branch"] = json!(branch);
    call(db, who, Method::POST, "/api/rate-policies", Some(body)).await
}

/// The `rate_policy` view's `policy` as `who` sees it on `branch`.
async fn current_policy(db: &TestDb, who: &str, branch: &str) -> Value {
    let a = call(
        db,
        who,
        Method::GET,
        &format!("/api/v3/views/rate_policy?branch={branch}"),
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "read rate_policy: {}", a.text);
    a.body["data"]["policy"].clone()
}

/// The guards of the frame OWNER enters first: USD `[-1, 1]` %, JPY tighter on one side.
fn frame_guards() -> Value {
    json!({
        "USD": {
            "buy": { "min_pct": "-1.0000", "max_pct": "1.0000" },
            "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
        },
        "JPY": {
            "buy": { "min_pct": "-2.0000", "max_pct": "0.5000" },
            "sell": { "min_pct": "-0.5000", "max_pct": "2.0000" }
        }
    })
}

/// The first frame of a branch, as OWNER enters it.
fn frame() -> Value {
    json!({
        "buyAdjustPct": "0",
        "sellAdjustPct": "0",
        "guards": frame_guards(),
        "maxReferenceAgeHours": 24,
        "onStale": "LIMIT",
        "staleLimitThb": "5000.00",
        "reason": "the frame for the test",
    })
}

/// A save that keeps the current frame exactly as the `rate_policy` view answered it and moves
/// only the two percents, as the A02 screen's "adjust all" sends it.
fn percents_only(current: &Value, buy: &str, sell: &str, reason: &str) -> Value {
    json!({
        "buyAdjustPct": buy,
        "sellAdjustPct": sell,
        "guards": current["guards"],
        "maxReferenceAgeHours": current["maxReferenceAgeHours"],
        "onStale": current["onStale"],
        "staleLimitThb": current["staleLimitThb"],
        "reason": reason,
    })
}

/// OWNER enters version 1 of `branch` and the test checks it was written as the first.
async fn owner_enters_the_frame(db: &TestDb, branch: &str) {
    let a = save(db, OWNER, branch, frame()).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "OWNER enters the frame: {}",
        a.text
    );
    assert_eq!(a.body["version"], json!(1), "the first version: {}", a.text);
    let v1 = db.version(branch, 1).await;
    assert_eq!(v1["approved_by"], json!(OWNER_ID), "{v1}");
    assert_eq!(
        v1["source_version"],
        Value::Null,
        "a branch's first version replaces none: {v1}"
    );
    assert_eq!(v1["current"], json!(true), "{v1}");
}

fn assert_refused_as_outside_the_role(a: &Answer, what: &str) {
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(
        a.status.as_u16(),
        err.status(),
        "{what}: the status of RoleNotAllowedHere; body {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["code"],
        json!(V3Error::from(err).code),
        "{what}: refused as a role outside its row; body {}",
        a.text
    );
}

// ------------------------------------------------------------ tests

/// OR-4b (#767, D-4, design §2 and §02 "Server ตรวจ … branch scope"): an employee who names a
/// branch outside their scope is refused 403 `outOfScope`, where v3 answered their own branch.
fn is_out_of_scope(a: &Answer, what: &str) {
    assert_eq!(a.status.as_u16(), 403, "{what}: status; body {}", a.body);
    assert_eq!(
        a.body["error"]["code"],
        serde_json::json!("outOfScope"),
        "{what}: {}",
        a.body
    );
}

#[actix_web::test]
async fn rates_may_change_the_percents_and_the_save_is_a_new_version_bound_to_the_one_it_replaces()
{
    let db = TestDb::start().await;
    owner_enters_the_frame(&db, B01).await;
    let v1_before = db.version(B01, 1).await;

    let current = current_policy(&db, RATES, B01).await;
    let a = save(
        &db,
        RATES,
        B01,
        percents_only(&current, "-0.5", "1.25", "BoT moved this morning"),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "RATES moves the %: {}", a.text);
    assert_eq!(a.body["version"], json!(2), "a new version: {}", a.text);
    assert!(a.body["id"].is_string(), "the new version's id: {}", a.text);

    let v2 = db.version(B01, 2).await;
    assert_eq!(v2["buy_adjust_pct"], json!("-0.5000"), "{v2}");
    assert_eq!(v2["sell_adjust_pct"], json!("1.2500"), "{v2}");
    assert_eq!(
        v2["guards"],
        frame_guards(),
        "the guards are the frame's: {v2}"
    );
    assert_eq!(v2["max_reference_age_hours"], json!(24), "{v2}");
    assert_eq!(v2["on_stale"], json!("LIMIT"), "{v2}");
    assert_eq!(v2["stale_limit_thb"], json!("5000.00"), "{v2}");
    assert_eq!(
        v2["approved_by"],
        json!(RATES_ID),
        "approved_by is the saver: {v2}"
    );
    assert_eq!(v2["reason"], json!("BoT moved this morning"), "{v2}");
    assert_eq!(
        v2["source_version"],
        json!(1),
        "the version it replaces: {v2}"
    );
    assert_eq!(v2["valid_from_set"], json!(true), "{v2}");
    assert_eq!(v2["current"], json!(true), "{v2}");

    let mut v1 = db.version(B01, 1).await;
    assert_eq!(
        v1["current"],
        json!(false),
        "the replaced version ends: {v1}"
    );
    v1["current"] = json!(true);
    assert_eq!(
        v1, v1_before,
        "nothing of version 1 but its end changed (immutable)"
    );
    assert_eq!(db.current_count(B01).await, 1, "one current version");

    let seen = current_policy(&db, RATES, B01).await;
    assert_eq!(
        seen["version"],
        json!(2),
        "the view answers the new version: {seen}"
    );
    assert_eq!(seen["buyAdjustPct"], json!("-0.5000"), "{seen}");
    assert_eq!(seen["approvedBy"], json!(RATES_ID), "{seen}");
    db.finish().await;
}

#[actix_web::test]
async fn a_rates_save_that_moves_a_guard_or_a_stale_setting_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    owner_enters_the_frame(&db, B01).await;
    let current = current_policy(&db, RATES, B01).await;
    let keep = percents_only(&current, "0.5", "0.5", "trying to move the frame");

    let mut wider = frame_guards();
    wider["USD"]["sell"]["max_pct"] = json!("5.0000");
    let mut one_more = frame_guards();
    one_more["EUR"] = frame_guards()["USD"].clone();
    let mut one_less = frame_guards();
    one_less.as_object_mut().expect("guards").remove("JPY");

    let changes: Vec<(&str, &str, Value)> = vec![
        ("a guard widened", "guards", wider),
        ("a guard added", "guards", one_more),
        ("a guard removed", "guards", one_less),
        ("all guards dropped", "guards", json!({})),
        ("the stale age moved", "maxReferenceAgeHours", json!(48)),
        ("the stale age dropped", "maxReferenceAgeHours", Value::Null),
        ("on_stale to STOP", "onStale", json!("STOP")),
        ("on_stale dropped", "onStale", Value::Null),
        ("the stale cap raised", "staleLimitThb", json!("9000.00")),
        ("the stale cap dropped", "staleLimitThb", Value::Null),
    ];
    let before = db.versions_of(B01).await;
    let records: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_records")
        .fetch_one(&db.pool)
        .await
        .expect("count idempotency_records");
    for (what, field, value) in changes {
        let mut body = keep.clone();
        body[field] = value;
        let a = save(&db, RATES, B01, body).await;
        assert_refused_as_outside_the_role(&a, what);
        assert_eq!(
            db.versions_of(B01).await,
            before,
            "{what}: no new version and no valid_to moved"
        );
    }
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_records")
        .fetch_one(&db.pool)
        .await
        .expect("count idempotency_records");
    assert_eq!(after, records, "a refused save keeps no answer");

    // the same caller, keeping the frame, is let through: the refusals were about the frame
    let a = save(&db, RATES, B01, keep).await;
    assert_eq!(a.status, StatusCode::OK, "only the %: {}", a.text);
    assert_eq!(a.body["version"], json!(2), "{}", a.text);
    db.finish().await;
}

#[actix_web::test]
async fn the_owner_may_change_the_guards_and_the_stale_settings() {
    let db = TestDb::start().await;
    owner_enters_the_frame(&db, B01).await;
    let mut guards = frame_guards();
    guards["USD"]["sell"]["max_pct"] = json!("2.5000");
    let a = save(
        &db,
        OWNER,
        B01,
        json!({
            "buyAdjustPct": "0.1",
            "sellAdjustPct": "-0.1",
            "guards": guards,
            "maxReferenceAgeHours": 12,
            "onStale": "STOP",
            "staleLimitThb": null,
            "reason": "a wider sell frame",
        }),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "OWNER moves the frame: {}",
        a.text
    );
    assert_eq!(a.body["version"], json!(2), "{}", a.text);
    let v2 = db.version(B01, 2).await;
    assert_eq!(v2["guards"], guards, "{v2}");
    assert_eq!(v2["max_reference_age_hours"], json!(12), "{v2}");
    assert_eq!(v2["on_stale"], json!("STOP"), "{v2}");
    assert_eq!(v2["stale_limit_thb"], Value::Null, "{v2}");
    assert_eq!(v2["approved_by"], json!(OWNER_ID), "{v2}");
    assert_eq!(v2["source_version"], json!(1), "{v2}");
    assert_eq!(db.current_count(B01).await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn a_save_without_a_reason_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    owner_enters_the_frame(&db, B01).await;
    let current = current_policy(&db, RATES, B01).await;
    let before = db.versions_of(B01).await;
    for (who, reason) in [(OWNER, ""), (OWNER, "   "), (RATES, ""), (RATES, "  ")] {
        let a = save(&db, who, B01, percents_only(&current, "0.2", "0.2", reason)).await;
        assert_eq!(
            a.status,
            StatusCode::BAD_REQUEST,
            "{who} with reason {reason:?}: {}",
            a.text
        );
        assert_eq!(
            a.body["error"]["code"],
            json!("validationFailed"),
            "{who}: {}",
            a.text
        );
        assert_eq!(
            a.body["error"]["field"],
            json!("reason"),
            "{who}: {}",
            a.text
        );
        assert_eq!(db.versions_of(B01).await, before, "{who}: nothing written");
    }
    db.finish().await;
}

#[actix_web::test]
async fn rates_cannot_save_on_a_branch_with_no_policy_yet_the_first_frame_is_the_owners() {
    let db = TestDb::start().await;
    let a = save(
        &db,
        RATES,
        B01,
        percents_only(&frame(), "0.5", "0.5", "a first frame by RATES"),
    )
    .await;
    assert_refused_as_outside_the_role(&a, "RATES with no frame to keep");
    assert!(db.versions_of(B01).await.is_empty(), "nothing written");

    owner_enters_the_frame(&db, B01).await;
    db.finish().await;
}

#[actix_web::test]
async fn a_rates_member_of_another_branch_cannot_save_this_branchs_policy() {
    let db = TestDb::start().await;
    owner_enters_the_frame(&db, B01).await;
    let b01 = current_policy(&db, OWNER, B01).await;
    let b01_before = db.versions_of(B01).await;
    let body = percents_only(&b01, "0.75", "0.75", "naming XC-B01 from XC-B09");

    // OR-4b (#767, D-4, §02 "Server ตรวจ … branch scope"): naming XC-B01, a branch outside the
    // member's scope, is refused as out of scope, whether XC-B09 is framed or not
    let a = save(&db, RATES_OF_B09, B01, body.clone()).await;
    is_out_of_scope(&a, "RATES of XC-B09 naming XC-B01, XC-B09 unframed");
    assert_eq!(db.versions_of(B01).await, b01_before, "XC-B01 untouched");
    assert!(db.versions_of(B09).await.is_empty(), "XC-B09 untouched");

    // XC-B09 with a frame of its own: XC-B01's frame is not XC-B09's, so still refused
    let mut b09_frame = frame();
    b09_frame["guards"] = json!({ "USD": frame_guards()["USD"] });
    let o = save(&db, OWNER, B09, b09_frame).await;
    assert_eq!(o.status, StatusCode::OK, "OWNER frames XC-B09: {}", o.text);
    let b09_before = db.versions_of(B09).await;
    let a = save(&db, RATES_OF_B09, B01, body).await;
    is_out_of_scope(&a, "RATES of XC-B09 naming XC-B01, XC-B09 framed");
    assert_eq!(db.versions_of(B01).await, b01_before, "XC-B01 untouched");
    assert_eq!(db.versions_of(B09).await, b09_before, "XC-B09 untouched");
    db.finish().await;
}

#[actix_web::test]
async fn every_save_is_a_new_version_chained_to_the_one_before() {
    let db = TestDb::start().await;
    owner_enters_the_frame(&db, B01).await;
    let c1 = current_policy(&db, RATES, B01).await;
    let a = save(&db, RATES, B01, percents_only(&c1, "0.1", "0.2", "second")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let c2 = current_policy(&db, OWNER, B01).await;
    let a = save(&db, OWNER, B01, percents_only(&c2, "0.3", "0.4", "third")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["version"], json!(3), "{}", a.text);

    let chain: Vec<(Value, Value, Value)> = {
        let mut out = Vec::new();
        for n in 1..=3 {
            let v = db.version(B01, n).await;
            out.push((
                v["source_version"].clone(),
                v["approved_by"].clone(),
                v["current"].clone(),
            ));
        }
        out
    };
    assert_eq!(
        chain,
        vec![
            (Value::Null, json!(OWNER_ID), json!(false)),
            (json!(1), json!(RATES_ID), json!(false)),
            (json!(2), json!(OWNER_ID), json!(true)),
        ],
        "source_version chains each version to the one it replaces; only the last is current"
    );
    db.finish().await;
}
