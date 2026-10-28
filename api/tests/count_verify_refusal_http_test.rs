//! Staff corrections follow-up (#564 issuecomment-6460939428, item 1a; design §8 gap 1): a policy refusal inside
//! `countVerify` answers its own code (`outOfScope`, 403) like `countReject` does, not `validationFailed`.
//!
//! Written before the implementation by the sub-controller (claude/studio-claude-4). Do not edit, delete or weaken
//! these tests; add your own in another file. Needs `TEST_DATABASE_URL`; nothing here skips.

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
        let schema = unique("cntreject");
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

async fn open_count_body(room: &Room) -> Value {
    json!({
        "branch": B01,
        "kind": "OPEN",
        "lines": [
            { "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "2" },
            { "currency": "THB", "denominationId": room.denomination("THB", "100").await, "qty": "3" }
        ]
    })
}

/// A waiting OPEN count of B01 submitted by `teller`; answers its id.
async fn waiting(room: &Room, teller: &Who) -> String {
    let a = post(
        room,
        teller,
        "/api/cash-counts",
        open_count_body(room).await,
    )
    .await;
    id_of(&a, "countSubmit")
}

async fn reject(room: &Room, who: &Who, count: &str, reason: &str) -> Answer {
    post(
        room,
        who,
        &format!("/api/cash-counts/{count}/reject"),
        json!({ "branch": B01, "reason": reason }),
    )
    .await
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

fn ids(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|r| r["id"].as_str().unwrap_or("").to_owned())
        .collect()
}

// A manager of another branch passes the seam on their own branch, then the count's branch refuses them.
#[actix_web::test]
async fn a_manager_of_another_branch_verifying_is_403_out_of_scope() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let elsewhere = room.person(Role::BranchManager, Some(B02)).await;
    let count = waiting(&room, &teller).await;

    let a = post(
        &room,
        &elsewhere,
        &format!("/api/cash-counts/{count}/verify"),
        json!({ "branch": B02 }),
    )
    .await;
    refused(
        &a,
        StatusCode::FORBIDDEN,
        "outOfScope",
        None,
        "a manager of another branch verifies",
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM cash_counts WHERE id = '{count}'::uuid AND verified_by IS NULL"
        ))
        .await,
        1,
        "unchanged"
    );
    room.finish().await;
}
