//! S3 (#3406, design `docs/design/blueprint/section-b-gaps.md` §S3, amends `org-roles-branches.md` §6) up-front tests:
//! A04 — a branch's contact channels and status, saved by `branchSave` (`PUT /api/branches/profile`) and answered by
//! the `admin_branches_profile` view.
//!
//! - B1: an existing branch, after the migration, answers `contactChannels: []` and `status: "ACTIVE"`.
//! - B2: two channels (PHONE, LINE) and INACTIVE are answered by the save and by the view, in the order saved; a
//!   value is stored trimmed.
//! - B3: an unknown kind, 11 channels, a 201-character value, an empty (all-space) value or a status outside
//!   ACTIVE/INACTIVE answers 400 `validationFailed` naming the field, and nothing is saved.
//! - B3 limits: exactly 10 channels and a 200-character value are accepted.
//!
//! Every test fails on `main` by its result (the fields are absent, or the bad save answers 200), never by a
//! compile error. The harness is `v3_org_admin_test.rs`'s. Needs `TEST_DATABASE_URL`; nothing skips.
//! B4 (a save without the two fields) is not here on purpose: it waits for the controller's ruling (#3406).

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
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ADMIN: &str = "ADMIN@*";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
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
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("s3contact");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .execute(&pool)
            .await
            .expect("insert the branches");
        Self { pool, schema }
    }

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

/// `x-test-as: <ROLE>@<branch>` (or `@*` for organisation-wide): one Blueprint role row.
struct HeaderUsers;

fn user_id(spec: &str) -> String {
    format!("user-{}", spec.replace(['@', '*'], "-"))
}

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        xchange::presentation::pages::SEEDED_XC_ORG_ID,
        vec![ScopedRole::new(
            role,
            branch,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

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

enum Key {
    Fresh,
}

async fn call(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
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
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
        match key {
            Key::Fresh => req = req.insert_header(("idempotency-key", unique("key"))),
        }
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn post(db: &TestDb, who: &str, method: Method, uri: &str, body: Value, key: Key) -> Answer {
    call(db, who, method, uri, Some(body), key).await
}

async fn get(db: &TestDb, who: &str, uri: &str) -> Answer {
    call(db, who, Method::GET, uri, None, Key::Fresh).await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

const PROFILE: &str = "/api/branches/profile";

fn view_uri() -> String {
    format!("/api/v3/views/admin_branches_profile?branch={B01}")
}

/// Today's `branchSave` body (`v3_org_admin_test.rs`'s `profile`) plus the two S3 fields.
fn save_body(channels: Value, status: &str) -> Value {
    json!({
        "branch": B01,
        "code": "SILOM",
        "address": "1 Silom Rd",
        "timezone": "Asia/Bangkok",
        "openingHours": { "mon": [{ "open": "09:00", "close": "17:00" }] },
        "bookingEnabled": true,
        "contactChannels": channels,
        "status": status
    })
}

fn channel(kind: &str, value: &str) -> Value {
    json!({ "kind": kind, "value": value })
}

async fn saved_profile(db: &TestDb) -> Value {
    let r = get(db, ADMIN, &view_uri()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.body["data"].clone()
}

fn assert_refused(a: &Answer, field: &str, case: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{case}: {}", a.text);
    assert_eq!(code(a), "validationFailed", "{case}: {}", a.text);
    assert_eq!(
        a.body["error"]["field"],
        json!(field),
        "{case}: names the field: {}",
        a.text
    );
}

/// B1: a branch that existed before the migration answers the defaults.
#[actix_web::test]
async fn s3_b1_an_existing_branch_has_no_channels_and_is_active() {
    let db = TestDb::start().await;
    let data = saved_profile(&db).await;
    assert_eq!(
        data["contactChannels"],
        json!([]),
        "B1 contactChannels: {data}"
    );
    assert_eq!(data["status"], json!("ACTIVE"), "B1 status: {data}");
    db.finish().await;
}

/// B2: two channels and INACTIVE come back from the save and the view, in the order saved, values trimmed.
#[actix_web::test]
async fn s3_b2_saved_channels_and_status_read_back_in_order() {
    let db = TestDb::start().await;
    let a = post(
        &db,
        ADMIN,
        Method::PUT,
        PROFILE,
        save_body(
            json!([
                channel("PHONE", "  +66 2 000 0000 "),
                channel("LINE", "@xchange")
            ]),
            "INACTIVE",
        ),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let want_channels = json!([
        { "kind": "PHONE", "value": "+66 2 000 0000" },
        { "kind": "LINE", "value": "@xchange" }
    ]);
    assert_eq!(
        a.body["contactChannels"], want_channels,
        "B2 the save answers the channels: {}",
        a.text
    );
    assert_eq!(
        a.body["status"],
        json!("INACTIVE"),
        "B2 the save answers the status: {}",
        a.text
    );
    let data = saved_profile(&db).await;
    assert_eq!(
        data["contactChannels"], want_channels,
        "B2 the view, in the order saved: {data}"
    );
    assert_eq!(
        data["status"],
        json!("INACTIVE"),
        "B2 the view's status: {data}"
    );
    assert_eq!(
        data["code"],
        json!("SILOM"),
        "B2 the rest of the profile is saved too: {data}"
    );
    db.finish().await;
}

/// B3: each bad value is refused 400 `validationFailed` naming its field, and nothing is saved.
#[actix_web::test]
async fn s3_b3_bad_channels_or_status_are_refused_and_nothing_is_saved() {
    let db = TestDb::start().await;
    let eleven: Vec<Value> = (0..11)
        .map(|i| channel("PHONE", &format!("0{i}")))
        .collect();
    let cases: Vec<(&str, Value, &str, &str)> = vec![
        (
            "an unknown kind",
            json!([channel("FAX", "02 000 0000")]),
            "ACTIVE",
            "contactChannels",
        ),
        (
            "11 channels",
            Value::Array(eleven),
            "ACTIVE",
            "contactChannels",
        ),
        (
            "a 201-character value",
            json!([channel("WEBSITE", &"w".repeat(201))]),
            "ACTIVE",
            "contactChannels",
        ),
        (
            "an all-space value",
            json!([channel("EMAIL", "   ")]),
            "ACTIVE",
            "contactChannels",
        ),
        ("a status outside the two", json!([]), "CLOSED", "status"),
    ];
    for (case, channels, status, field) in cases {
        let mut body = save_body(channels, status);
        body["code"] = json!("REFUSED");
        let a = post(&db, ADMIN, Method::PUT, PROFILE, body, Key::Fresh).await;
        assert_refused(&a, field, case);
        let data = saved_profile(&db).await;
        assert_ne!(
            data["code"],
            json!("REFUSED"),
            "{case}: nothing is saved: {data}"
        );
    }
    db.finish().await;
}

/// B3's limits are inclusive: 10 channels and a 200-character value are accepted and read back.
#[actix_web::test]
async fn s3_b3_ten_channels_and_a_200_character_value_are_accepted() {
    let db = TestDb::start().await;
    let mut ten: Vec<Value> = (0..9).map(|i| channel("PHONE", &format!("0{i}"))).collect();
    ten.push(channel("WEBSITE", &"w".repeat(200)));
    let want = Value::Array(ten.clone());
    let a = post(
        &db,
        ADMIN,
        Method::PUT,
        PROFILE,
        save_body(Value::Array(ten), "ACTIVE"),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let data = saved_profile(&db).await;
    assert_eq!(
        data["contactChannels"], want,
        "10 channels, the last 200 characters: {data}"
    );
    db.finish().await;
}

/// Every kind the design names is accepted (`section-b-gaps.md` §S3).
#[actix_web::test]
async fn s3_every_named_kind_is_accepted() {
    let db = TestDb::start().await;
    let all = json!([
        channel("PHONE", "02 000 0000"),
        channel("LINE", "@xchange"),
        channel("EMAIL", "desk@example.test"),
        channel("FACEBOOK", "xchange.th"),
        channel("WEBSITE", "https://example.test")
    ]);
    let a = post(
        &db,
        ADMIN,
        Method::PUT,
        PROFILE,
        save_body(all.clone(), "ACTIVE"),
        Key::Fresh,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let data = saved_profile(&db).await;
    assert_eq!(data["contactChannels"], all, "the five kinds: {data}");
    db.finish().await;
}
