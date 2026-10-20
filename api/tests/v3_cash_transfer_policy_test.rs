//! Ticket #1371 (CA-8c): `transferPolicySave` (`PUT /api/cash-transfer-policy`) and the `cash_transfer_policy`
//! view, design `docs/design/blueprint/cash-shift-transfer.md` §4.4 (`:223`), §4.7 (`:325`) and §10 (`:708`).
//!
//! Written before the implementation by the test-writer claude/709ec749, on the BE lane controller claude/ad05bf8d's
//! order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Everything goes over HTTP through the whole route table, so a missing route or view fails on its answer, not on a
//! compile error. The callers are Blueprint `access::Actor`s (organisation-wide roles, real `users` rows) handed to the
//! seams through `SessionReader::callers`; the legacy sessions are empty, so nothing here builds the v2 `Actor` that
//! #1030 retires from the seams. Who may call is computed from `RoleGrants`, never from a role name.
//!
//! Choices of this file, each marked where used:
//! - The command carries `branch` in its body, the optional field every cash command's seam reads (#624 G1).
//! - A refusal is pinned by its wire code, not its HTTP status.
//! - `TooManyDecimals` reaches the wire as the cash commands' `validationFailed` with `CashError::TooManyDecimals`'s
//!   sentence (`commands/cash.rs::from_cash_error`, `:125-146`).
//! - A permission refusal is the policy's `roleNotAllowedHere`, as for every CA-8 command and view (#624 G2a,
//!   `v3_cash_commands_test.rs:11`). The ticket's §8 calls it "roleNotAllowed"; object on #1371 if that differs.
//! - The version's format is not pinned (#1371 §7 [proposed]): two saves give two different non-empty versions.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test` (never the queue's port 6381); nothing
//! skips. Each test works in a schema of its own.

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
use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Permission, Role, RoleGrants, ScopedRole};
use xc_cash::domain::error::CashError;
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ROUTE: &str = "/api/cash-transfer-policy";
const VIEW: &str = "cash_transfer_policy";
const B01: &str = "XC-B01";
/// The seeded organisation (`20261011100000_organizations.sql`).
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// A second organisation, made by each test that needs it.
const ORG2: &str = "00000000-0000-0000-0000-0000000000b2";
const B2: &str = "OT-B01";
/// A permission refusal on an `EntryPath::Rpc` row (#624 G2a).
const NOT_ALLOWED: &str = "roleNotAllowedHere";

/// The Blueprint staff roles, used only to *make* actors.
const STAFF_ROLES: [Role; 8] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
    Role::VerificationReviewer,
];

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn holds(role: Role) -> bool {
    RoleGrants::seed(role).contains(&Permission::CashTransferPolicySet)
}

/// A role holding `CashTransferPolicySet` (EXECUTIVE and FINANCE under §11.2).
fn a_setter() -> Role {
    *STAFF_ROLES
        .iter()
        .find(|r| holds(**r))
        .expect("some staff role holds CashTransferPolicySet")
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("v3cashpolicy");
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
            .expect("apply api/migrations");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(B01)
            .execute(&pool)
            .await
            .expect("insert a branch");
        Self { pool, schema }
    }

    /// A second organisation with one branch.
    async fn other_org(&self) {
        sqlx::query(
            "INSERT INTO organizations (id, code, legal_name, timezone, created_at) \
             VALUES ($1::uuid, 'OT', NULL, 'Asia/Bangkok', now())",
        )
        .bind(ORG2)
        .execute(&self.pool)
        .await
        .expect("a second organisation");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(B2)
            .bind(ORG2)
            .execute(&self.pool)
            .await
            .expect("the second organisation's branch");
    }

    /// An organisation-wide user of `org` with `role`: a `users` row and its `user_roles` row.
    async fn person(&self, role: Role, org: &str) -> Who {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let id = seed_person(
            &mut self.pool.acquire().await.unwrap(),
            org,
            &unique(role.as_str()),
            &[RoleRow {
                role_code: role.as_str(),
                branch_id: None,
                valid_from: from,
                valid_to: None,
            }],
        )
        .await
        .expect("insert a user");
        Who {
            spec: format!("{id}|{}|{org}", role.as_str()),
            id,
        }
    }

    async fn rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM cash_transfer_policy_versions")
            .fetch_one(&self.pool)
            .await
            .expect("count the policy versions")
    }

    /// `(organization_id, limits, set_by)` of every policy row, oldest first.
    async fn stored(&self) -> Vec<(String, Value, String)> {
        sqlx::query_as::<_, (String, Value, String)>(
            "SELECT organization_id::text, limits, set_by::text FROM cash_transfer_policy_versions \
             ORDER BY created_at, version",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read the policy versions")
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

/// A signed-in person: their user id and the spec that signs them in.
struct Who {
    id: String,
    spec: String,
}

/// The `access::Actor` of a spec `<user id>|<ROLE>|<organisation>`, organisation-wide.
fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, org) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        org,
        vec![ScopedRole::new(role, None::<BranchId>, from, None)],
        None,
    ))
}

struct HeaderUsers;

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

/// One request through the whole route table as `who`.
async fn call(
    room: &Room,
    who: &Who,
    method: Method,
    uri: &str,
    body: Option<(Value, &str)>,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
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
        .insert_header((AS, who.spec.clone()));
    if let Some((body, key)) = body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// `PUT /api/cash-transfer-policy {branch, limits}` as `who` under `key`.
async fn save(room: &Room, who: &Who, branch: &str, limits: Value, key: &str) -> Answer {
    call(
        room,
        who,
        Method::PUT,
        ROUTE,
        Some((json!({ "branch": branch, "limits": limits }), key)),
    )
    .await
}

/// The `cash_transfer_policy` view as `who`, named at `branch`.
async fn read(room: &Room, who: &Who, branch: &str) -> Answer {
    call(
        room,
        who,
        Method::GET,
        &format!("/api/v3/views/{VIEW}?branch={branch}"),
        None,
    )
    .await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"].as_str().unwrap_or("").to_owned()
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body.clone()
}

/// The `data` of a view answer: present, and `null` or the version.
fn data(a: &Answer, what: &str) -> Value {
    let body = ok(a, what);
    assert!(
        body.as_object().is_some_and(|o| o.contains_key("data")),
        "{what}: a view answer with `data`: {}",
        a.text
    );
    body["data"].clone()
}

/// The `version` of a saved policy: a non-empty string (its format is not pinned).
fn version_of(v: &Value, what: &str) -> String {
    let version = v["version"]
        .as_str()
        .unwrap_or_else(|| panic!("{what}: a `version` string: {v}"));
    assert!(!version.is_empty(), "{what}: a non-empty version");
    version.to_owned()
}

// ------------------------------------------------------------ the command

/// Case 1: a holder of `CashTransferPolicySet` saves; one row with the caller's organisation, the limits as strings
/// and `set_by` = the caller; the answer is `{version, limits, setBy, createdAt}`.
#[actix_web::test]
async fn a_setter_saves_one_version_with_their_org_the_limits_as_strings_and_themselves_as_set_by()
{
    let room = Room::start().await;
    let exec = room.person(a_setter(), ORG).await;
    let a = save(
        &room,
        &exec,
        B01,
        json!({ "USD": "300.00" }),
        &unique("key"),
    )
    .await;
    let saved = ok(&a, "the save");
    version_of(&saved, "the save");
    assert_eq!(saved["limits"], json!({ "USD": "300.00" }), "{}", a.text);
    assert_eq!(saved["setBy"], json!(exec.id), "{}", a.text);
    assert!(
        saved["createdAt"]
            .as_str()
            .is_some_and(|s| s.parse::<DateTime<Utc>>().is_ok()),
        "createdAt is a timestamp: {}",
        a.text
    );
    assert_eq!(
        room.stored().await,
        vec![(ORG.to_owned(), json!({ "USD": "300.00" }), exec.id.clone())],
        "exactly one row: the caller's organisation, the limits as strings, set_by the caller"
    );
    room.finish().await;
}

/// Case 2: every staff role without `CashTransferPolicySet` (BRANCH_MANAGER and ADMIN among them) is refused, and
/// writes nothing.
#[actix_web::test]
async fn every_role_without_the_permission_is_refused_and_writes_nothing() {
    let room = Room::start().await;
    let refused: Vec<Role> = STAFF_ROLES.iter().copied().filter(|r| !holds(*r)).collect();
    assert!(refused.contains(&Role::BranchManager) && refused.contains(&Role::Admin));
    for role in refused {
        let who = room.person(role, ORG).await;
        let a = save(&room, &who, B01, json!({ "USD": "300.00" }), &unique("key")).await;
        assert_eq!(code(&a), NOT_ALLOWED, "{}: {}", role.as_str(), a.text);
    }
    assert_eq!(room.rows().await, 0, "no refused save wrote a row");
    room.finish().await;
}

/// Case 3: more decimal places than the currency's minor unit is `TooManyDecimals`, and writes nothing.
#[actix_web::test]
async fn an_amount_finer_than_the_currencys_minor_unit_is_too_many_decimals_and_writes_nothing() {
    let room = Room::start().await;
    let exec = room.person(a_setter(), ORG).await;
    let a = save(&room, &exec, B01, json!({ "USD": "1.001" }), &unique("key")).await;
    assert_eq!(code(&a), "validationFailed", "{}", a.text);
    assert_eq!(
        a.body["error"]["message"].as_str(),
        Some(CashError::TooManyDecimals.to_string().as_str()),
        "the refusal is TooManyDecimals: {}",
        a.text
    );
    assert_eq!(room.rows().await, 0);
    room.finish().await;
}

/// Case 4: the same `Idempotency-Key` with the same body replays the first answer and writes no second row.
#[actix_web::test]
async fn the_same_key_and_body_replays_the_first_answer_and_writes_one_row() {
    let room = Room::start().await;
    let exec = room.person(a_setter(), ORG).await;
    let key = unique("key");
    let first = save(&room, &exec, B01, json!({ "USD": "300.00" }), &key).await;
    let first = ok(&first, "the first save");
    let again = save(&room, &exec, B01, json!({ "USD": "300.00" }), &key).await;
    assert_eq!(
        ok(&again, "the replay"),
        first,
        "the replay is the first answer"
    );
    assert_eq!(room.rows().await, 1, "one row after the replay");
    room.finish().await;
}

// ------------------------------------------------------------ the view

/// Case 5: no version yet → `null`.
#[actix_web::test]
async fn the_view_answers_null_before_any_save() {
    let room = Room::start().await;
    let exec = room.person(a_setter(), ORG).await;
    let a = read(&room, &exec, B01).await;
    assert_eq!(data(&a, "the view"), Value::Null, "{}", a.text);
    room.finish().await;
}

/// Case 6: after two saves the view answers the second (newest by `created_at`); the two versions differ.
#[actix_web::test]
async fn after_two_saves_the_view_answers_the_second_and_the_versions_differ() {
    let room = Room::start().await;
    let exec = room.person(a_setter(), ORG).await;
    let first = save(
        &room,
        &exec,
        B01,
        json!({ "USD": "300.00" }),
        &unique("key"),
    )
    .await;
    let first = version_of(&ok(&first, "the first save"), "the first save");
    let second = save(
        &room,
        &exec,
        B01,
        json!({ "USD": "500.00", "EUR": "20.00" }),
        &unique("key"),
    )
    .await;
    let second = version_of(&ok(&second, "the second save"), "the second save");
    assert_ne!(first, second, "two saves, two versions");

    let a = read(&room, &exec, B01).await;
    let v = data(&a, "the view");
    assert_eq!(
        v["version"],
        json!(second),
        "the newest version: {}",
        a.text
    );
    assert_eq!(
        v["limits"],
        json!({ "USD": "500.00", "EUR": "20.00" }),
        "{}",
        a.text
    );
    assert_eq!(v["setBy"], json!(exec.id), "{}", a.text);
    room.finish().await;
}

/// Case 7: another organisation's version is never answered, even when it is the newest row in the table.
#[actix_web::test]
async fn the_view_never_answers_another_organisations_version() {
    let room = Room::start().await;
    room.other_org().await;
    let exec = room.person(a_setter(), ORG).await;
    let other = room.person(a_setter(), ORG2).await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by, created_at) \
         VALUES ('other-org', $1::uuid, jsonb_build_object('USD', '9.00'), $2::uuid, now() + interval '1 hour')",
    )
    .bind(ORG2)
    .bind(&other.id)
    .execute(&room.pool)
    .await
    .expect("another organisation's newest version");

    let a = read(&room, &exec, B01).await;
    assert_eq!(data(&a, "before any own save"), Value::Null, "{}", a.text);

    let mine = save(
        &room,
        &exec,
        B01,
        json!({ "USD": "300.00" }),
        &unique("key"),
    )
    .await;
    let mine = version_of(&ok(&mine, "the save"), "the save");
    let a = read(&room, &exec, B01).await;
    let v = data(&a, "after the own save");
    assert_eq!(v["version"], json!(mine), "{}", a.text);
    assert_eq!(v["limits"], json!({ "USD": "300.00" }), "{}", a.text);
    room.finish().await;
}

/// Case 8: every staff role without `CashTransferPolicySet` is refused the view.
#[actix_web::test]
async fn the_view_is_refused_to_every_role_without_the_permission() {
    let room = Room::start().await;
    for role in STAFF_ROLES.iter().copied().filter(|r| !holds(*r)) {
        let who = room.person(role, ORG).await;
        let a = read(&room, &who, B01).await;
        assert_eq!(code(&a), NOT_ALLOWED, "{}: {}", role.as_str(), a.text);
    }
    room.finish().await;
}
