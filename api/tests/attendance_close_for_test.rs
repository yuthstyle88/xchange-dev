//! Staff corrections §4 A, §5 and §10 (`docs/design/staff-corrections.md`): `attendanceCloseFor`
//! `POST /api/attendance/{id}/close` and the `open_attendance` view, through the whole route table.
//!
//! Written before the implementation by the sub-controller (SC-0, claude/studio-claude-4). Do not edit, delete or
//! weaken these tests; add your own in another file. Needs `TEST_DATABASE_URL`; nothing here skips.
//!
//! The command reads the clock (`Utc::now()`), so every time here is relative to now: a forgotten clock-in is
//! clocked in through `/api/attendance` and then moved two days back in `attendance.data`.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const UNKNOWN_UUID: &str = "00000000-0000-0000-0000-00000000c0a8";
const B02: &str = "XC-B02";

static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

async fn lg4_fund(
    pool: &PgPool,
    from: xc_cash::domain::posting::CashPlace,
    to: xc_cash::domain::posting::CashPlace,
    currency: &str,
    amount: &str,
) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from,
                to,
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "lg4-fund:{currency}:{}",
                        LG4_FUND.fetch_add(1, Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund through the ledger");
    tx.commit().await.expect("commit");
}

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
        let schema = unique("closefor");
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

        for branch in [B01, B02] {
            sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("insert branch");
        }
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase) \
             VALUES ($1, 'OPEN')",
        )
        .bind(B01)
        .execute(&pool)
        .await
        .expect("branch safe");

        let safe = || xc_cash::domain::posting::CashPlace::Safe(BranchId::new(B01));
        lg4_fund(
            &pool,
            xc_cash::domain::posting::CashPlace::Bank,
            safe(),
            "USD",
            "500.00",
        )
        .await;
        lg4_fund(
            &pool,
            xc_cash::domain::posting::CashPlace::Bank,
            safe(),
            "THB",
            "1000000.00",
        )
        .await;

        Self { pool, schema }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        self.person_with(&[role], branch).await
    }

    /// One user holding every role in `roles` on `branch`.
    async fn person_with(&self, roles: &[Role], branch: Option<&str>) -> Who {
        let rows: Vec<xc_testdb::people::RoleRow> = roles
            .iter()
            .map(|r| xc_testdb::people::RoleRow {
                role_code: r.as_str(),
                branch_id: branch,
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            })
            .collect();
        let id = xc_testdb::people::seed_person(
            &mut self.pool.acquire().await.expect("acquire"),
            ORG,
            &unique("person"),
            &rows,
        )
        .await
        .expect("insert a user and give them their roles");
        let codes: Vec<&str> = roles.iter().map(|r| r.as_str()).collect();
        Who {
            spec: format!("{id}|{}|{}", codes.join(","), branch.unwrap_or("*")),
            id,
        }
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
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

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let roles = role
        .split(',')
        .map(|r| Role::parse(r).map(|r| ScopedRole::new(r, branch.clone(), from, None)))
        .collect::<Option<Vec<_>>>()?;
    Some(User::new(id, ORG, roles, None))
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
        Box::pin(async move { test_callers::callers(None, user_of(&spec)) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn post(room: &Room, who: &Who, uri: &str, body: Value) -> Answer {
    send(room, who, Method::POST, uri, Some(body)).await
}

async fn get(room: &Room, who: &Who, uri: &str) -> Answer {
    send(room, who, Method::GET, uri, None).await
}

async fn send(room: &Room, who: &Who, method: Method, uri: &str, body: Option<Value>) -> Answer {
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
        .insert_header((AS, who.spec.clone()))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN));
    if let Some(body) = body {
        req = req
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

fn code(a: &Answer) -> String {
    a.body["error"]["code"].as_str().unwrap_or("").to_owned()
}

fn field(a: &Answer) -> Option<String> {
    a.body["error"]["field"].as_str().map(str::to_owned)
}

/// `status` and `code` of a refusal, with its `field`.
fn refused(a: &Answer, status: StatusCode, want: &str, want_field: Option<&str>, what: &str) {
    assert_eq!(a.status, status, "{what}: {}", a.text);
    assert_eq!(code(a), want, "{what}: {}", a.text);
    assert_eq!(field(a).as_deref(), want_field, "{what}: {}", a.text);
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body.clone()
}

fn id_of(a: &Answer, what: &str) -> String {
    ok(a, what)["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn rfc(ms: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(ms)
        .expect("a time")
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// `who` clocks in through `/api/attendance`; the row is then moved `days_back` days into the past.
/// Answers `(attendance id, clock-in ms)`.
async fn clocked_in(room: &Room, who: &Who, days_back: i64) -> (String, i64) {
    let a = post(
        room,
        who,
        "/api/attendance",
        json!({ "branch": B01, "direction": "IN" }),
    )
    .await;
    ok(&a, "clock IN");
    let id = room
        .text(&format!(
            "SELECT id FROM attendance WHERE branch_id = '{B01}' AND employee_id = '{}' \
             AND data->>'clockOut' IS NULL",
            who.id
        ))
        .await
        .expect("the open row");
    let clock_in = now_ms() - days_back * DAY_MS;
    room.scalar(&format!(
        "WITH u AS (UPDATE attendance SET data = jsonb_set(data, '{{clockIn}}', to_jsonb({clock_in}::bigint)) \
         WHERE id = '{id}' RETURNING 1) SELECT count(*) FROM u"
    ))
    .await;
    (id, clock_in)
}

async fn close(room: &Room, who: &Who, id: &str, body: Value) -> Answer {
    post(room, who, &format!("/api/attendance/{id}/close"), body).await
}

fn body(clock_out: &str, reason: &str) -> Value {
    json!({ "branch": B01, "clockOut": clock_out, "reason": reason })
}

fn rows(a: &Answer, what: &str) -> Vec<Value> {
    ok(a, what)["data"]
        .as_array()
        .unwrap_or_else(|| {
            panic!(
                "{what}: a paged view answers `data` as the rows array: {}",
                a.text
            )
        })
        .clone()
}

// A11 over HTTP.
#[actix_web::test]
async fn a_manager_closes_a_forgotten_clock_in() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let (id, clock_in) = clocked_in(&room, &teller, 2).await;
    let out = rfc(clock_in + 9 * 60 * 60 * 1000);

    let a = close(&room, &manager, &id, body(&out, "ลืมลงเวลาออก")).await;

    let b = ok(&a, "attendanceCloseFor");
    assert_eq!(b["id"], id.as_str(), "{}", a.text);
    assert_eq!(b["employeeId"], teller.id.as_str(), "{}", a.text);
    assert_eq!(b["clockIn"], rfc(clock_in).as_str(), "{}", a.text);
    assert_eq!(b["clockOut"], out.as_str(), "{}", a.text);
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM attendance WHERE id = '{id}' AND (data->>'clockOut')::bigint = {}",
            clock_in + 9 * 60 * 60 * 1000
        ))
        .await,
        1,
        "the row holds the entered clock-out"
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM ops_events WHERE branch_id = '{B01}' AND kind = 'CLOCK_OUT_FOR' \
             AND actor = '{}' AND details->>'reason' = 'ลืมลงเวลาออก' AND details->>'employeeId' = '{}'",
            manager.id, teller.id
        ))
        .await,
        1,
        "one CLOCK_OUT_FOR by the manager"
    );
    room.finish().await;
}

// A1: a teller lacks AttendanceCloseFor, so the seam refuses the route.
#[actix_web::test]
async fn a_teller_is_refused_by_the_seam() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let other = room.person(Role::Teller, Some(B01)).await;
    let (id, clock_in) = clocked_in(&room, &teller, 2).await;
    let a = close(&room, &other, &id, body(&rfc(clock_in + 3_600_000), "why")).await;
    refused(
        &a,
        StatusCode::BAD_REQUEST,
        "roleNotAllowedHere",
        None,
        "a teller",
    );
    room.finish().await;
}

// A1/A2: a manager of B02 cannot reach B01's row: refused on B01, not found on B02.
#[actix_web::test]
async fn another_branchs_manager_cannot_close_it() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let elsewhere = room.person(Role::BranchManager, Some(B02)).await;
    let (id, clock_in) = clocked_in(&room, &teller, 2).await;
    let out = rfc(clock_in + 3_600_000);

    let a = close(&room, &elsewhere, &id, body(&out, "why")).await;
    assert!(
        a.status == StatusCode::FORBIDDEN || a.status == StatusCode::BAD_REQUEST,
        "on B01: refused, {}",
        a.text
    );
    assert!(
        ["outOfScope", "roleNotAllowedHere"].contains(&code(&a).as_str()),
        "on B01: {}",
        a.text
    );
    let a = close(
        &room,
        &elsewhere,
        &id,
        json!({ "branch": B02, "clockOut": out, "reason": "why" }),
    )
    .await;
    refused(&a, StatusCode::NOT_FOUND, "notFound", None, "on B02");
    room.finish().await;
}

// A2
#[actix_web::test]
async fn an_unknown_row_is_404() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let a = close(
        &room,
        &manager,
        "no-such-row",
        body(&rfc(now_ms() - DAY_MS), "why"),
    )
    .await;
    refused(&a, StatusCode::NOT_FOUND, "notFound", None, "unknown");
    room.finish().await;
}

// A3, A4
#[actix_web::test]
async fn self_close_and_not_open_are_409() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let (own, own_in) = clocked_in(&room, &manager, 2).await;
    let a = close(&room, &manager, &own, body(&rfc(own_in + 3_600_000), "why")).await;
    refused(&a, StatusCode::CONFLICT, "selfClose", None, "own row");

    let (id, clock_in) = clocked_in(&room, &teller, 2).await;
    ok(
        &close(
            &room,
            &manager,
            &id,
            body(&rfc(clock_in + 3_600_000), "why"),
        )
        .await,
        "first close",
    );
    let a = close(
        &room,
        &manager,
        &id,
        body(&rfc(clock_in + 7_200_000), "again"),
    )
    .await;
    refused(&a, StatusCode::CONFLICT, "notOpen", None, "closed already");
    room.finish().await;
}

// A5–A10, each with its field.
#[actix_web::test]
async fn each_input_fault_names_its_field() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let (id, clock_in) = clocked_in(&room, &teller, 2).await;
    let good = rfc(clock_in + 3_600_000);

    let cases: Vec<(&str, Value, StatusCode, &str, &str)> = vec![
        (
            "blank reason",
            body(&good, "  "),
            StatusCode::BAD_REQUEST,
            "reasonRequired",
            "reason",
        ),
        (
            "no reason",
            json!({ "branch": B01, "clockOut": good }),
            StatusCode::BAD_REQUEST,
            "reasonRequired",
            "reason",
        ),
        (
            "501 units",
            body(&good, &"ก".repeat(501)),
            StatusCode::BAD_REQUEST,
            "validationFailed",
            "reason",
        ),
        (
            "no clockOut",
            json!({ "branch": B01, "reason": "why" }),
            StatusCode::BAD_REQUEST,
            "validationFailed",
            "clockOut",
        ),
        (
            "not RFC 3339",
            body("yesterday 6pm", "why"),
            StatusCode::BAD_REQUEST,
            "validationFailed",
            "clockOut",
        ),
        (
            "before clock-in",
            body(&rfc(clock_in - 60_000), "why"),
            StatusCode::BAD_REQUEST,
            "beforeClockIn",
            "clockOut",
        ),
        (
            "in the future",
            body(&rfc(now_ms() + DAY_MS), "why"),
            StatusCode::BAD_REQUEST,
            "inFuture",
            "clockOut",
        ),
        (
            "today",
            body(&rfc(now_ms() - 1_000), "why"),
            StatusCode::BAD_REQUEST,
            "notEarlierDay",
            "clockOut",
        ),
    ];
    for (what, b, status, want, want_field) in cases {
        let a = close(&room, &manager, &id, b).await;
        refused(&a, status, want, Some(want_field), what);
    }
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM attendance WHERE id = '{id}' AND data->>'clockOut' IS NULL"
        ))
        .await,
        1,
        "still open after every refusal"
    );
    room.finish().await;
}

// §8 note 4: the view lists other people's open rows from earlier days, and only those.
#[actix_web::test]
async fn open_attendance_lists_what_a_manager_may_close() {
    let room = Room::start().await;
    let forgot = room.person(Role::Teller, Some(B01)).await;
    let today = room.person(Role::Teller, Some(B01)).await;
    let closed = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let (old, old_in) = clocked_in(&room, &forgot, 2).await;
    clocked_in(&room, &today, 0).await;
    let (shut, shut_in) = clocked_in(&room, &closed, 3).await;
    ok(
        &close(
            &room,
            &manager,
            &shut,
            body(&rfc(shut_in + 3_600_000), "why"),
        )
        .await,
        "close one",
    );
    clocked_in(&room, &manager, 2).await;

    let a = get(
        &room,
        &manager,
        &format!("/api/v3/views/open_attendance?branch={B01}"),
    )
    .await;
    let r = rows(&a, "open_attendance");
    assert_eq!(
        r.len(),
        1,
        "only the forgotten row of someone else: {}",
        a.text
    );
    assert_eq!(r[0]["id"], old.as_str());
    assert_eq!(r[0]["employeeId"], forgot.id.as_str());
    assert_eq!(r[0]["clockIn"], rfc(old_in).as_str());
    let name = room
        .text(&format!(
            "SELECT name FROM users WHERE id = '{}'::uuid",
            forgot.id
        ))
        .await
        .expect("a name");
    assert_eq!(r[0]["name"], name.as_str());

    let a = get(
        &room,
        &today,
        &format!("/api/v3/views/open_attendance?branch={B01}"),
    )
    .await;
    assert!(
        !a.status.is_success(),
        "a teller cannot read it: {}",
        a.text
    );
    assert_eq!(code(&a), "roleNotAllowedHere", "{}", a.text);
    room.finish().await;
}
