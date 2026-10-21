//! EX-5 (#1075): A01's alert thresholds — `GET /api/v3/views/branch_alert_settings?branch=` and
//! `POST /api/v3/commands/branch_alert_settings_save` (design `docs/design/blueprint/executive-finance.md` §6.1,
//! §8.2; owner decisions E-3 and E-6). Written before the implementation by the ticket author (claude/89bd61d9).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Shapes these tests assume (ticket §7–§8, [required] JSON):
//! - View: `{ branch, staleDataMinutes: number|null, floors: [{currency, minQuantity}], version: number|null }`;
//!   `version: null`, `staleDataMinutes: null` and no floors for a branch with no row.
//! - Save body: `{ branch, staleDataMinutes, floors, expectedVersion }` + `Idempotency-Key`; the answer is the saved
//!   set with its new `version`. `minQuantity` is a decimal string, at most 4 places.
//! - `staleDataMinutes` ≤ 0 → `400 validationFailed`, field `staleDataMinutes`. A currency not in `currencies`, a
//!   duplicate currency, or a negative or malformed `minQuantity` → `400 validationFailed`, field `floors`.
//! - `expectedVersion` ≠ the row's `version` (or not `null` without a row, or `null` with one) → `409 staleVersion`,
//!   nothing written.
//! - One `audit_events` row per save: `action = 'BRANCH_ALERT_SETTINGS_SAVE'`, `object_ref = 'branch:<id>'`,
//!   `before_redacted`/`after_redacted` = the threshold sets.
//! - `updated_by` = the caller's user id.
//! - Holders of `AlertSettingsSave` (EXECUTIVE, ADMIN; EX-1 #1070) may read and save; any other role is refused
//!   `roleNotAllowedHere`, as the policy refuses a role outside its row (#945's tests). A branch outside the caller's
//!   scope is refused `outOfScope`.
//!
//! Callers are Blueprint `access::Actor`s with exactly the role under test, through `SessionReader::callers`, as in
//! `v3_org_admin_test.rs`; their user ids are uuids and their organisation is the database's `XC`, so `updated_by`
//! and the organisation key can be written as the real ones are.
//!
//! Needs `TEST_DATABASE_URL`; nothing skips.

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
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Permission, Role, RoleGrants, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const SAVE: &str = "/api/v3/commands/branch_alert_settings_save";
const EXECUTIVE: &str = "EXECUTIVE@*";
// #4155 W/A1: AlertSettingsSave (DASHBOARD FULL) is held by EXECUTIVE only in the seed;
// second saver is a second EXECUTIVE user (with branch scope XC-B01).
const SECOND_EXECUTIVE: &str = "EXECUTIVE@XC-B01";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn view(branch: &str) -> String {
    format!("/api/v3/views/branch_alert_settings?branch={branch}")
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
    org: String,
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
        let schema = unique("ex5alerts");
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
            .max_connections(8)
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
        let org: String =
            sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = 'XC-B01'")
                .fetch_one(&pool)
                .await
                .expect("the branches' organisation");
        Self { pool, schema, org }
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn settings_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM branch_alert_settings")
            .await
    }

    async fn floor_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM branch_stock_floors").await
    }

    async fn audits(&self) -> i64 {
        self.count("SELECT count(*) FROM audit_events WHERE action = 'BRANCH_ALERT_SETTINGS_SAVE'")
            .await
    }

    async fn finish(self) {
        let TestDb { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

/// `x-test-as: <ROLE>@<branch>` (or `@*` for organisation-wide): one Blueprint role row of the database's
/// organisation, with a uuid user id.
struct HeaderUsers {
    org: String,
}

/// A fixed uuid per caller spec, so `updated_by` can be checked.
fn user_id(spec: &str) -> String {
    let n = match spec {
        EXECUTIVE => 1,
        SECOND_EXECUTIVE => 2, // #4155 W/A1: second EXECUTIVE user id
        "ADMIN@XC-B01" => 3,
        "FINANCE@XC-B01" => 4,
        "TELLER@XC-B01" => 5,
        "BRANCH_MANAGER@XC-B01" => 6,
        "RATE_MANAGER@XC-B01" => 7,
        "VERIFICATION_REVIEWER@XC-B01" => 8,
        "MARKETING@XC-B01" => 9,
        other => panic!("no user id written for {other}"),
    };
    format!("00000000-0000-4000-8000-{n:012}")
}

fn user_of(spec: &str, org: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "FINANCE" => Role::Finance,
        "ADMIN" => Role::Admin,
        "EXECUTIVE" => Role::Executive,
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" => Role::Teller,
        "RATE_MANAGER" => Role::RateManager,
        "VERIFICATION_REVIEWER" => Role::VerificationReviewer,
        "MARKETING" => Role::Marketing,
        other => panic!("unknown role {other}"),
    };
    let branch = (branch != "*").then(|| BranchId::new(branch));
    Some(User::new(
        &user_id(spec),
        org,
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
        let org = self.org.clone();
        Box::pin(async move {
            let user = user_of(&spec, &org);
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

async fn call(
    db: &TestDb,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: &str,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers {
        org: db.org.clone(),
    });
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

async fn save(db: &TestDb, who: &str, body: Value) -> Answer {
    call(db, who, Method::POST, SAVE, Some(body), &unique("key")).await
}

async fn save_with_key(db: &TestDb, who: &str, body: Value, key: &str) -> Answer {
    call(db, who, Method::POST, SAVE, Some(body), key).await
}

async fn read(db: &TestDb, who: &str, branch: &str) -> Answer {
    call(db, who, Method::GET, &view(branch), None, "").await
}

/// The `data` of a 200 from the view.
async fn current(db: &TestDb, who: &str, branch: &str) -> Value {
    let a = read(db, who, branch).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "read {branch} as {who}: {}",
        a.text
    );
    a.body["data"].clone()
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn body(
    branch: &str,
    minutes: Option<i64>,
    floors: &[(&str, &str)],
    expected: Option<i64>,
) -> Value {
    json!({
        "branch": branch,
        "staleDataMinutes": minutes,
        "floors": floors.iter().map(|(c, q)| json!({ "currency": c, "minQuantity": q })).collect::<Vec<_>>(),
        "expectedVersion": expected,
    })
}

fn decimal(v: &Value) -> Decimal {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("minQuantity is a decimal string: {v}"));
    Decimal::from_str(s).unwrap_or_else(|e| panic!("minQuantity {s:?} parses: {e}"))
}

/// The set a view or a save answers, with `minQuantity` compared as numbers.
fn assert_set(
    got: &Value,
    branch: &str,
    minutes: Option<i64>,
    floors: &[(&str, &str)],
    version: Option<i64>,
) {
    assert_eq!(got["branch"], json!(branch), "{got}");
    assert_eq!(got["staleDataMinutes"], json!(minutes), "{got}");
    assert_eq!(got["version"], json!(version), "{got}");
    let listed = got["floors"]
        .as_array()
        .unwrap_or_else(|| panic!("floors is an array: {got}"));
    assert_eq!(listed.len(), floors.len(), "{got}");
    for (currency, quantity) in floors {
        let floor = listed
            .iter()
            .find(|f| f["currency"] == json!(currency))
            .unwrap_or_else(|| panic!("a floor for {currency}: {got}"));
        assert_eq!(
            decimal(&floor["minQuantity"]),
            Decimal::from_str(quantity).expect("a test decimal"),
            "{currency}: {got}"
        );
    }
}

fn assert_refused(a: &Answer, status: StatusCode, want: &str, what: &str) {
    assert_eq!(a.status, status, "{what}: status; body {}", a.text);
    assert_eq!(code(a), want, "{what}: {}", a.text);
}

fn assert_invalid(a: &Answer, field: &str, what: &str) {
    assert_refused(a, StatusCode::BAD_REQUEST, "validationFailed", what);
    assert_eq!(a.body["error"]["field"], json!(field), "{what}: {}", a.text);
}

// ------------------------------------------------------------ reading and saving (E-3, E-6)

/// §7: a branch with no row reads as no thresholds and no version.
#[actix_web::test]
async fn a_branch_with_no_row_reads_empty_with_no_version() {
    let db = TestDb::start().await;
    let got = current(&db, EXECUTIVE, B01).await;
    assert_set(&got, B01, None, &[], None);
    db.finish().await;
}

/// §9 (E-6) / #4155 W/A1: EXECUTIVE saves, a second EXECUTIVE user saves after it (only EXECUTIVE holds
/// `AlertSettingsSave` in the seed); each reads back the same set and its version.
#[actix_web::test]
async fn executive_and_finance_both_save_and_read_back_the_same_set() {
    let db = TestDb::start().await;

    let first = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "1000"), ("JPY", "50000.5")], None),
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "EXECUTIVE saves: {}",
        first.text
    );
    assert_set(
        &first.body,
        B01,
        Some(30),
        &[("USD", "1000"), ("JPY", "50000.5")],
        Some(1),
    );
    assert_set(
        &current(&db, EXECUTIVE, B01).await,
        B01,
        Some(30),
        &[("USD", "1000"), ("JPY", "50000.5")],
        Some(1),
    );

    // #4155 W/A1: second saver is a second EXECUTIVE user.
    let second = save(
        &db,
        SECOND_EXECUTIVE,
        body(B01, Some(45), &[("USD", "800.1234")], Some(1)),
    )
    .await;
    assert_eq!(
        second.status,
        StatusCode::OK,
        "SECOND_EXECUTIVE saves: {}",
        second.text
    );
    assert_set(&second.body, B01, Some(45), &[("USD", "800.1234")], Some(2));
    assert_set(
        &current(&db, SECOND_EXECUTIVE, B01).await,
        B01,
        Some(45),
        &[("USD", "800.1234")],
        Some(2),
    );
    db.finish().await;
}

/// §7 [required]: `updated_by` is the caller's user id, on the settings row and its floors.
#[actix_web::test]
async fn the_saver_is_recorded_as_updated_by() {
    let db = TestDb::start().await;
    // #4155 W/A1: second EXECUTIVE user id recorded as updated_by.
    let a = save(
        &db,
        SECOND_EXECUTIVE,
        body(B01, Some(30), &[("USD", "10")], None),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let want = user_id(SECOND_EXECUTIVE);
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM branch_alert_settings WHERE branch_id = '{B01}' AND updated_by = '{want}'::uuid"
        ))
        .await,
        1
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM branch_stock_floors WHERE branch_id = '{B01}' AND updated_by = '{want}'::uuid"
        ))
        .await,
        1
    );
    db.finish().await;
}

/// §8: `staleDataMinutes` may be `null` (E-3: no row value means no stale-data alert) while floors are set.
#[actix_web::test]
async fn null_minutes_with_floors_is_a_valid_set() {
    let db = TestDb::start().await;
    let a = save(&db, EXECUTIVE, body(B01, None, &[("EUR", "200")], None)).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_set(
        &current(&db, EXECUTIVE, B01).await,
        B01,
        None,
        &[("EUR", "200")],
        Some(1),
    );
    db.finish().await;
}

/// §9: `floors: []` removes the branch's floors.
#[actix_web::test]
async fn an_empty_floor_list_removes_the_branchs_floors() {
    let db = TestDb::start().await;
    let a = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "1000"), ("EUR", "5")], None),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let b = save(&db, EXECUTIVE, body(B01, Some(30), &[], Some(1))).await;
    assert_eq!(b.status, StatusCode::OK, "{}", b.text);
    assert_set(
        &current(&db, EXECUTIVE, B01).await,
        B01,
        Some(30),
        &[],
        Some(2),
    );
    assert_eq!(db.floor_rows().await, 0, "the floors are gone");
    db.finish().await;
}

// ------------------------------------------------------------ who may (E-6, scope)

/// §9 / §11.2: roles without `AlertSettingsSave` in `RoleGrants::seed(role)` are refused both routes, and nothing is written.
#[actix_web::test]
async fn other_roles_are_refused_and_nothing_is_written() {
    let db = TestDb::start().await;
    let candidates = [
        ("EXECUTIVE@*", Role::Executive),
        ("ADMIN@XC-B01", Role::Admin),
        ("RATE_MANAGER@XC-B01", Role::RateManager),
        ("BRANCH_MANAGER@XC-B01", Role::BranchManager),
        ("TELLER@XC-B01", Role::Teller),
        ("FINANCE@XC-B01", Role::Finance),
        ("MARKETING@XC-B01", Role::Marketing),
        ("VERIFICATION_REVIEWER@XC-B01", Role::VerificationReviewer),
    ];
    assert!(
        candidates
            .map(|(_, r)| r)
            .iter()
            .any(|r| !RoleGrants::seed(*r).contains(&Permission::AlertSettingsSave)),
        "§11.2 must leave some role without AlertSettingsSave, or this test refuses nobody"
    );
    for (who, _) in candidates
        .into_iter()
        .filter(|(_, r)| !RoleGrants::seed(*r).contains(&Permission::AlertSettingsSave))
    {
        let s = save(&db, who, body(B01, Some(30), &[("USD", "10")], None)).await;
        assert_ne!(s.status, StatusCode::OK, "{who} saves: {}", s.text);
        assert_eq!(code(&s), "roleNotAllowedHere", "{who} saves: {}", s.text);
        let r = read(&db, who, B01).await;
        assert_eq!(code(&r), "roleNotAllowedHere", "{who} reads: {}", r.text);
    }
    assert_eq!(db.settings_rows().await, 0);
    assert_eq!(db.floor_rows().await, 0);
    assert_eq!(db.audits().await, 0);
    db.finish().await;
}

/// §8 / #4155 W/A1: a branch outside the caller's scope is refused `outOfScope`: a branch-scoped EXECUTIVE of XC-B01
/// (holder of `AlertSettingsSave` under #4155 W/A1) naming XC-B09.
#[actix_web::test]
async fn a_branch_outside_the_callers_scope_is_refused() {
    let db = TestDb::start().await;
    let own = save(&db, SECOND_EXECUTIVE, body(B01, Some(30), &[], None)).await;
    assert_eq!(own.status, StatusCode::OK, "its own branch: {}", own.text);

    let other = save(&db, SECOND_EXECUTIVE, body(B09, Some(30), &[], None)).await;
    assert_refused(
        &other,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "another branch's save",
    );
    let r = read(&db, SECOND_EXECUTIVE, B09).await;
    assert_refused(
        &r,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "another branch's read",
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM branch_alert_settings WHERE branch_id = '{B09}'"
        ))
        .await,
        0
    );
    db.finish().await;
}

// ------------------------------------------------------------ the version check (§6.1, no last-write-wins)

/// §9: a stale `expectedVersion` is `staleVersion`, writes nothing and audits nothing. So is `null` once a row
/// exists, and a number before one does.
#[actix_web::test]
async fn a_stale_expected_version_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    let early = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "1")], Some(1)),
    )
    .await;
    assert_refused(
        &early,
        StatusCode::CONFLICT,
        "staleVersion",
        "a version before any row",
    );
    assert_eq!(db.settings_rows().await, 0);

    let first = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "1000")], None),
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert_eq!(db.audits().await, 1);

    for (expected, what) in [
        (Some(7), "a wrong version"),
        (None, "null once the row exists"),
    ] {
        // #4155 W/A1: second saver is SECOND_EXECUTIVE.
        let a = save(
            &db,
            SECOND_EXECUTIVE,
            body(B01, Some(99), &[("EUR", "5")], expected),
        )
        .await;
        assert_refused(&a, StatusCode::CONFLICT, "staleVersion", what);
    }
    assert_set(
        &current(&db, EXECUTIVE, B01).await,
        B01,
        Some(30),
        &[("USD", "1000")],
        Some(1),
    );
    assert_eq!(db.audits().await, 1, "a refused save writes no audit");
    db.finish().await;
}

/// §9 / §6.1: two first saves at once, both `expectedVersion: null` — one creates the row, the other is
/// `staleVersion`.
#[actix_web::test]
async fn two_first_saves_at_once_give_one_row_and_one_stale_refusal() {
    let db = TestDb::start().await;
    // #4155 W/A1: second saver is SECOND_EXECUTIVE.
    let (a, b) = tokio::join!(
        save(&db, EXECUTIVE, body(B01, Some(30), &[("USD", "1")], None)),
        save(
            &db,
            SECOND_EXECUTIVE,
            body(B01, Some(60), &[("EUR", "2")], None)
        ),
    );
    let mut codes = vec![
        if a.status == StatusCode::OK {
            "ok".to_owned()
        } else {
            code(&a)
        },
        if b.status == StatusCode::OK {
            "ok".to_owned()
        } else {
            code(&b)
        },
    ];
    codes.sort();
    assert_eq!(
        codes,
        ["ok", "staleVersion"],
        "a: {} / b: {}",
        a.text,
        b.text
    );
    assert_eq!(db.settings_rows().await, 1);
    assert_eq!(
        current(&db, EXECUTIVE, B01).await["version"],
        json!(1),
        "one save, version 1"
    );
    assert_eq!(db.audits().await, 1);
    db.finish().await;
}

// ------------------------------------------------------------ validation (§8 step 1)

/// §9: `staleDataMinutes` of 0 or −5 is `validationFailed` on that field, and nothing is written.
#[actix_web::test]
async fn non_positive_minutes_are_refused() {
    let db = TestDb::start().await;
    for minutes in [0, -5] {
        let a = save(&db, EXECUTIVE, body(B01, Some(minutes), &[], None)).await;
        assert_invalid(&a, "staleDataMinutes", &format!("{minutes} minutes"));
    }
    assert_eq!(db.settings_rows().await, 0);
    assert_eq!(db.audits().await, 0);
    db.finish().await;
}

/// §9 and §8: a floor for a currency not in `currencies`, a duplicate currency, a negative, malformed or
/// five-place `minQuantity` is `validationFailed` on `floors`, and nothing is written.
#[actix_web::test]
async fn bad_floors_are_refused() {
    let db = TestDb::start().await;
    let cases: [(&str, Vec<(&str, &str)>); 5] = [
        ("a currency not in the register", vec![("ZZZ", "10")]),
        ("a duplicate currency", vec![("USD", "10"), ("USD", "20")]),
        ("a negative quantity", vec![("USD", "-1")]),
        ("a malformed quantity", vec![("USD", "ten")]),
        ("five decimal places", vec![("USD", "1.23456")]),
    ];
    for (what, floors) in cases {
        let a = save(&db, EXECUTIVE, body(B01, Some(30), &floors, None)).await;
        assert_invalid(&a, "floors", what);
    }
    assert_eq!(db.settings_rows().await, 0);
    assert_eq!(db.floor_rows().await, 0);
    assert_eq!(db.audits().await, 0);
    db.finish().await;
}

// ------------------------------------------------------------ audit and idempotency

/// §9 / §6.1: each successful save writes exactly one audit row naming the branch, whose `before_redacted` is the
/// set it replaced and whose `after_redacted` is the set it wrote.
#[actix_web::test]
async fn each_save_writes_one_audit_row_with_the_sets() {
    let db = TestDb::start().await;
    let a = save(
        &db,
        EXECUTIVE,
        body(B01, Some(30), &[("USD", "1000")], None),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(db.audits().await, 1);
    // #4155 W/A1: second saver is SECOND_EXECUTIVE.
    let b = save(
        &db,
        SECOND_EXECUTIVE,
        body(B01, Some(45), &[("EUR", "5")], Some(1)),
    )
    .await;
    assert_eq!(b.status, StatusCode::OK, "{}", b.text);
    assert_eq!(db.audits().await, 2);

    let rows: Vec<(String, String, Option<Value>, Option<Value>)> = sqlx::query_as(
        "SELECT object_ref, actor_id, before_redacted, after_redacted FROM audit_events \
         WHERE action = 'BRANCH_ALERT_SETTINGS_SAVE' ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the audit rows");
    assert!(
        rows.iter().all(|r| r.0 == format!("branch:{B01}")),
        "{rows:?}"
    );
    let (first_after, second_before, second_after) = (
        rows[0].3.clone().expect("the first save's after set"),
        rows[1].2.clone().expect("the second save's before set"),
        rows[1].3.clone().expect("the second save's after set"),
    );
    assert_eq!(
        second_before, first_after,
        "the second save replaced what the first wrote"
    );
    assert_ne!(second_after, second_before, "and wrote a different set");
    let after_text = second_after.to_string();
    assert!(
        after_text.contains("45") && after_text.contains("EUR"),
        "{after_text}"
    );
    db.finish().await;
}

/// §9: the same `Idempotency-Key` twice is one save — one row at version 1, one audit row, the same answer.
#[actix_web::test]
async fn the_same_idempotency_key_twice_is_one_save() {
    let db = TestDb::start().await;
    let key = unique("ex5-key");
    let set = body(B01, Some(30), &[("USD", "1000")], None);
    let a = save_with_key(&db, EXECUTIVE, set.clone(), &key).await;
    let b = save_with_key(&db, EXECUTIVE, set, &key).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(b.status, StatusCode::OK, "the replay: {}", b.text);
    assert_eq!(a.body, b.body, "the replay answers the first save");
    assert_eq!(current(&db, EXECUTIVE, B01).await["version"], json!(1));
    assert_eq!(db.audits().await, 1);
    db.finish().await;
}
