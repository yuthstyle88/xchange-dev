//! Ticket #2499: `countSubmit` (`POST /api/cash-counts`) with a repeated `denominationId`
//! in `lines` answers 400 `validationFailed` and writes nothing, while distinct lines succeed.

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

static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("cntrepdenom");
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
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase) \
             VALUES ($1, 'OPEN')",
        )
        .bind(B01)
        .execute(&pool)
        .await
        .expect("the branch's safe");
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
        let id = xc_testdb::people::seed_person(
            &mut self.pool.acquire().await.expect("acquire"),
            ORG,
            &unique(role.as_str()),
            &[xc_testdb::people::RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("insert a user and give them a role");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
        }
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
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

async fn lg4_fund(
    pool: &sqlx::PgPool,
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

struct Who {
    #[allow(dead_code)]
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, branch, from, None)],
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
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.spec.clone()))
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

fn code(a: &Answer) -> String {
    a.body["error"]["code"].as_str().unwrap_or("").to_owned()
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

/// D1: THB 1000 × 2 and THB 1000 × 3 (same `denominationId`) -> 400 `validationFailed`;
/// no `cash_counts` row and no `cash_count_lines` row for it (main: 500).
#[actix_web::test]
async fn d1_repeated_denomination_id_is_refused_400_validation_failed_and_writes_nothing() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let d1000 = room.denomination("THB", "1000").await;

    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        json!({
            "branch": B01,
            "kind": "OPEN",
            "lines": [
                { "currency": "THB", "denominationId": d1000, "qty": "2" },
                { "currency": "THB", "denominationId": d1000, "qty": "3" }
            ]
        }),
    )
    .await;

    assert_eq!(a.status, StatusCode::BAD_REQUEST, "D1 status: {}", a.text);
    assert_eq!(code(&a), "validationFailed", "D1 code: {}", a.text);
    let msg = a.body["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains(&d1000.to_string()),
        "D1 error message must name denomination id {d1000}: {}",
        a.text
    );
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM cash_counts").await,
        0,
        "D1 must write no cash_counts row"
    );
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM cash_count_lines").await,
        0,
        "D1 must write no cash_count_lines row"
    );
    room.finish().await;
}

/// D2: THB 1000 × 2 and THB 100 × 3 (distinct) -> 200, as today: the count stored with both lines.
#[actix_web::test]
async fn d2_distinct_denominations_succeed_200_and_store_both_lines() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let d1000 = room.denomination("THB", "1000").await;
    let d100 = room.denomination("THB", "100").await;

    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        json!({
            "branch": B01,
            "kind": "OPEN",
            "lines": [
                { "currency": "THB", "denominationId": d1000, "qty": "2" },
                { "currency": "THB", "denominationId": d100, "qty": "3" }
            ]
        }),
    )
    .await;

    let count_id = id_of(&a, "D2 distinct denominations");
    assert_eq!(
        room.scalar(&format!(
            "SELECT COUNT(*) FROM cash_counts WHERE id = '{count_id}'"
        ))
        .await,
        1
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT COUNT(*) FROM cash_count_lines WHERE count_id = '{count_id}'"
        ))
        .await,
        2
    );
    room.finish().await;
}

/// D3: THB 1000 × 0 and THB 1000 × 2 (same id, one row zero) -> 400 `validationFailed`:
/// still a repeated row, so a zero doesn't make it acceptable.
#[actix_web::test]
async fn d3_repeated_denomination_with_zero_qty_row_is_refused_400_validation_failed() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let d1000 = room.denomination("THB", "1000").await;

    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        json!({
            "branch": B01,
            "kind": "OPEN",
            "lines": [
                { "currency": "THB", "denominationId": d1000, "qty": "0" },
                { "currency": "THB", "denominationId": d1000, "qty": "2" }
            ]
        }),
    )
    .await;

    assert_eq!(a.status, StatusCode::BAD_REQUEST, "D3 status: {}", a.text);
    assert_eq!(code(&a), "validationFailed", "D3 code: {}", a.text);
    let msg = a.body["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains(&d1000.to_string()),
        "D3 error message must name denomination id {d1000}: {}",
        a.text
    );
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM cash_counts").await,
        0,
        "D3 must write no cash_counts row"
    );
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM cash_count_lines").await,
        0,
        "D3 must write no cash_count_lines row"
    );
    room.finish().await;
}

/// D4: one line, THB 1000 × 2 -> 200.
#[actix_web::test]
async fn d4_single_line_succeeds_200() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let d1000 = room.denomination("THB", "1000").await;

    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        json!({
            "branch": B01,
            "kind": "OPEN",
            "lines": [
                { "currency": "THB", "denominationId": d1000, "qty": "2" }
            ]
        }),
    )
    .await;

    let count_id = id_of(&a, "D4 single line");
    assert_eq!(
        room.scalar(&format!(
            "SELECT COUNT(*) FROM cash_counts WHERE id = '{count_id}'"
        ))
        .await,
        1
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT COUNT(*) FROM cash_count_lines WHERE count_id = '{count_id}'"
        ))
        .await,
        1
    );
    room.finish().await;
}

#[actix_web::test]
async fn an_empty_lines_count_is_400_validation_failed_and_writes_nothing() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;

    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        json!({
            "branch": B01,
            "kind": "OPEN",
            "lines": []
        }),
    )
    .await;

    assert_eq!(
        a.status,
        StatusCode::BAD_REQUEST,
        "empty lines status: {}",
        a.text
    );
    assert_eq!(code(&a), "validationFailed", "empty lines code: {}", a.text);
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM cash_counts").await,
        0,
        "empty lines must write no cash_counts row"
    );
    room.finish().await;
}

