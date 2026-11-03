//! F4 and F6 (eb2849a5 on-screen report, #564 issuecomment-6596339013): every setting names who saved it, by name, from
//! its own row (design `docs/design/blueprint/user-set-settings.md` "Every setting names who saved it", rows S-1..S-7).
//!
//! - F6: `cash_transfer_policy` answered `setBy` as the raw user id and no name; `margin_guard` the same.
//! - F4: the drawer low levels had no setter of their own, so after `drawerLowLevelsSave` the page named whoever last
//!   saved the count-difference limit.
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own. The harness is `v3_cash_transfer_policy_test.rs`'s (a schema of its own, branch XC-B01, real
//! `users` rows through `seed_person`, organisation-wide callers). Needs `TEST_DATABASE_URL`; nothing skips.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

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

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
    Some(
        User::new(
            id,
            org,
            vec![ScopedRole::new(role, None::<BranchId>, from, None)],
            None,
        )
        .with_branch(Some(BranchId::new("MAIN"))),
    )
}

struct HeaderUsers;

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
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

// ------------------------------------------------------------ helpers of this file

/// Gives `who` the display name `name` (`users.name`, the column the setter name is read from).
async fn name_as(room: &Room, who: &Who, name: &str) {
    sqlx::query("UPDATE users SET name = $2 WHERE id = $1::uuid")
        .bind(&who.id)
        .bind(name)
        .execute(&room.pool)
        .await
        .expect("name the user");
}

async fn put(room: &Room, who: &Who, uri: &str, body: Value) -> Answer {
    call(room, who, Method::PUT, uri, Some((body, &unique("key")))).await
}

async fn view(room: &Room, who: &Who, name: &str) -> Answer {
    call(
        room,
        who,
        Method::GET,
        &format!("/api/v3/views/{name}?branch={B01}"),
        None,
    )
    .await
}

// ------------------------------------------------------------ F6: the transfer policy (S-1..S-3)

/// S-1: the Owner, named "Owner A", saves the transfer policy; the view answers `setBy` = the id and
/// `setByName` = "Owner A".
#[actix_web::test]
async fn s1_the_policy_view_names_its_setter() {
    let room = Room::start().await;
    let owner = room.person(Role::Executive, ORG).await;
    name_as(&room, &owner, "Owner A").await;
    ok(
        &save(
            &room,
            &owner,
            B01,
            json!({ "USD": "300.00" }),
            &unique("key"),
        )
        .await,
        "transferPolicySave",
    );

    let v = data(&read(&room, &owner, B01).await, VIEW);
    assert_eq!(v["setBy"], json!(owner.id), "{v}");
    assert_eq!(v["setByName"], json!("Owner A"), "{v}");
    room.finish().await;
}

/// S-2: `transferPolicySave`'s own answer carries the same `setByName`.
#[actix_web::test]
async fn s2_the_policy_save_answers_its_setters_name() {
    let room = Room::start().await;
    let owner = room.person(Role::Executive, ORG).await;
    name_as(&room, &owner, "Owner A").await;
    let a = save(
        &room,
        &owner,
        B01,
        json!({ "USD": "300.00" }),
        &unique("key"),
    )
    .await;
    let body = ok(&a, "transferPolicySave");
    assert_eq!(body["setBy"], json!(owner.id), "{}", a.text);
    assert_eq!(body["setByName"], json!("Owner A"), "{}", a.text);
    room.finish().await;
}

/// S-3: a setter whose `users.name` is blank answers `setByName: null`, never the id.
#[actix_web::test]
async fn s3_a_blank_name_answers_null_never_the_id() {
    let room = Room::start().await;
    let owner = room.person(Role::Executive, ORG).await;
    name_as(&room, &owner, "   ").await;
    let a = save(
        &room,
        &owner,
        B01,
        json!({ "USD": "300.00" }),
        &unique("key"),
    )
    .await;
    let body = ok(&a, "transferPolicySave");
    assert!(
        body.as_object()
            .is_some_and(|o| o.contains_key("setByName")),
        "the field is present: {}",
        a.text
    );
    assert_eq!(body["setByName"], Value::Null, "{}", a.text);

    let v = data(&read(&room, &owner, B01).await, VIEW);
    assert!(
        v.as_object().is_some_and(|o| o.contains_key("setByName")),
        "the field is present: {v}"
    );
    assert_eq!(v["setByName"], Value::Null, "{v}");
    assert_eq!(v["setBy"], json!(owner.id), "{v}");
    room.finish().await;
}

// ------------------------------------------------------------ F6: the margin guard (S-4)

/// S-4: `marginGuardSet` by the Owner, then the `margin_guard` view: `setByName` = the setter's name.
#[actix_web::test]
async fn s4_the_margin_guard_view_names_its_setter() {
    let room = Room::start().await;
    let owner = room.person(Role::Executive, ORG).await;
    name_as(&room, &owner, "Owner A").await;
    ok(
        &put(
            &room,
            &owner,
            "/api/margin-guard",
            json!({ "maxSpreadShare": "0.5000" }),
        )
        .await,
        "marginGuardSet",
    );

    let v = data(&view(&room, &owner, "margin_guard").await, "margin_guard");
    assert_eq!(v["setBy"], json!(owner.id), "{v}");
    assert_eq!(v["setByName"], json!("Owner A"), "{v}");
    room.finish().await;
}

// ------------------------------------------------------------ F4: the drawer low levels (S-5..S-7)

/// S-5: Finance's name on the count-difference limit, the Owner's on the drawer levels. The limit is saved by one
/// person and the levels by another; both the `drawerLowLevelsSave` answer and the `cash_limits` view keep them apart.
#[actix_web::test]
async fn s5_the_levels_name_their_own_setter_apart_from_the_limits() {
    let room = Room::start().await;
    let limit_setter = room.person(Role::Executive, ORG).await;
    name_as(&room, &limit_setter, "Limit Setter").await;
    let levels_setter = room.person(Role::Executive, ORG).await;
    name_as(&room, &levels_setter, "Levels Setter").await;

    ok(
        &put(
            &room,
            &limit_setter,
            "/api/cash-limits",
            json!({ "branch": B01, "countDifferenceLimitThb": "5000.00" }),
        )
        .await,
        "cashLimitsSave",
    );
    let a = put(
        &room,
        &levels_setter,
        "/api/cash-limits/drawer-low-levels",
        json!({ "branch": B01, "levels": [{ "currency": "THB", "lowLevel": "2000" }] }),
    )
    .await;
    let saved = ok(&a, "drawerLowLevelsSave");
    for (what, d) in [
        ("the drawerLowLevelsSave answer", saved),
        (
            "the cash_limits view",
            data(
                &view(&room, &limit_setter, "cash_limits").await,
                "cash_limits",
            ),
        ),
    ] {
        assert_eq!(d["updatedBy"], json!(limit_setter.id), "{what}: {d}");
        assert_eq!(d["updatedByName"], json!("Limit Setter"), "{what}: {d}");
        assert_eq!(
            d["drawerLowLevelsUpdatedBy"],
            json!(levels_setter.id),
            "{what}: {d}"
        );
        assert_eq!(
            d["drawerLowLevelsUpdatedByName"],
            json!("Levels Setter"),
            "{what}: {d}"
        );
        assert!(
            d["drawerLowLevelsUpdatedAt"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "{what}: the levels' own time: {d}"
        );
    }
    room.finish().await;
}

/// S-6: a branch with no drawer levels answers the three `drawerLowLevels*` setter fields as `null`.
#[actix_web::test]
async fn s6_no_levels_means_null_setter_fields() {
    let room = Room::start().await;
    let owner = room.person(Role::Executive, ORG).await;
    let d = data(&view(&room, &owner, "cash_limits").await, "cash_limits");
    for field in [
        "drawerLowLevelsUpdatedBy",
        "drawerLowLevelsUpdatedByName",
        "drawerLowLevelsUpdatedAt",
    ] {
        assert!(
            d.as_object().is_some_and(|o| o.contains_key(field)),
            "{field} is present: {d}"
        );
        assert_eq!(d[field], Value::Null, "{field}: {d}");
    }
    room.finish().await;
}

/// S-7: `cashLimitsSave` keeps today's `updatedBy` / `updatedByName` (pinned), and with no levels saved the levels'
/// setter stays `null`.
#[actix_web::test]
async fn s7_the_limits_setter_is_unchanged() {
    let room = Room::start().await;
    let owner = room.person(Role::Executive, ORG).await;
    name_as(&room, &owner, "Owner A").await;
    let a = put(
        &room,
        &owner,
        "/api/cash-limits",
        json!({ "branch": B01, "countDifferenceLimitThb": "5000.00" }),
    )
    .await;
    let d = ok(&a, "cashLimitsSave");
    assert_eq!(d["updatedBy"], json!(owner.id), "{}", a.text);
    assert_eq!(d["updatedByName"], json!("Owner A"), "{}", a.text);
    assert_eq!(d["drawerLowLevelsUpdatedByName"], Value::Null, "{}", a.text);
    room.finish().await;
}
