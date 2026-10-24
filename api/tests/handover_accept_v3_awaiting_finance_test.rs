//! Ticket #5063 (P7): v3 `handoverAccept` over the branch's count-difference limit answers
//! 200 `{id, status: "AWAITING_FINANCE"}` and commits the `COUNT_DIFFERENCE` approval request
//! instead of rolling it back on error.

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
    async fn start(limit_thb: &str) -> Self {
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
        let schema = unique("v3hoaf");
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
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
            .bind(B01)
            .execute(&pool)
            .await
            .expect("the branch's safe");
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, $3::numeric, 'test', now())",
        )
        .bind(ORG)
        .bind(B01)
        .bind(limit_thb)
        .execute(&pool)
        .await
        .expect("a count-difference limit");

        lg4_fund(
            &pool,
            xc_cash::domain::posting::CashPlace::Bank,
            xc_cash::domain::posting::CashPlace::Safe(BranchId::new(B01)),
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

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn count_difference_requests(&self, count_id: &str) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT id::text, status FROM approval_requests \
             WHERE type = 'COUNT_DIFFERENCE' AND object_id = $1",
        )
        .bind(count_id)
        .fetch_all(&self.pool)
        .await
        .expect("read approval requests")
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

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
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
        .set_payload(body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
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

#[actix_web::test]
async fn handover_accept_over_the_limit_answers_awaiting_finance_and_commits_the_request() {
    let room = Room::start("50").await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let thb1000 = room.denomination("THB", "1000").await;
    let thb100 = room.denomination("THB", "100").await;

    // Open teller's shift with THB 2,300 (1000 × 2 + 100 × 3).
    let open_count = id_of(
        &post(
            &room,
            &teller,
            "/api/cash-counts",
            json!({
                "branch": B01,
                "kind": "OPEN",
                "lines": [
                    { "currency": "THB", "denominationId": thb1000, "qty": "2" },
                    { "currency": "THB", "denominationId": thb100, "qty": "3" }
                ]
            }),
        )
        .await,
        "teller submits OPEN count",
    );
    ok(
        &post(
            &room,
            &manager,
            &format!("/api/cash-counts/{open_count}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "branch manager verifies OPEN count",
    );
    let shift = room
        .text(&format!(
            "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.id
        ))
        .await
        .expect("teller's open shift");

    // Start hand-over counting THB 2,200 (100 short, over the THB 50 limit).
    let handover_id = id_of(
        &post(
            &room,
            &teller,
            "/api/handovers",
            json!({
                "branch": B01,
                "fromShiftId": shift,
                "receiverId": receiver.id,
                "lines": [
                    { "currency": "THB", "denominationId": thb1000, "qty": "2" },
                    { "currency": "THB", "denominationId": thb100, "qty": "2" }
                ]
            }),
        )
        .await,
        "handoverStart",
    );
    let count_id = room
        .text(&format!(
            "SELECT count_id::text FROM handovers WHERE id = '{handover_id}'::uuid"
        ))
        .await
        .expect("hand-over's count_id");

    // First accept: answers 200 {id, status: "AWAITING_FINANCE"}, and commits one PENDING request.
    let first = post(
        &room,
        &receiver,
        &format!("/api/handovers/{handover_id}/accept"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(
        ok(&first, "first handoverAccept"),
        json!({ "id": handover_id, "status": "AWAITING_FINANCE" }),
        "first handoverAccept body: {}",
        first.text
    );
    let requests = room.count_difference_requests(&count_id).await;
    assert_eq!(
        requests.len(),
        1,
        "one COUNT_DIFFERENCE request committed: {requests:?}"
    );
    assert_eq!(requests[0].1, "PENDING");
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM handovers WHERE id = '{handover_id}'::uuid"
        ))
        .await
        .as_deref(),
        Some("PENDING"),
        "hand-over stays PENDING while awaiting FINANCE"
    );

    // Asking again: answers the same and adds no request.
    let second = post(
        &room,
        &receiver,
        &format!("/api/handovers/{handover_id}/accept"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(
        ok(&second, "second handoverAccept"),
        json!({ "id": handover_id, "status": "AWAITING_FINANCE" }),
        "second handoverAccept body: {}",
        second.text
    );
    let requests_after = room.count_difference_requests(&count_id).await;
    assert_eq!(
        requests_after, requests,
        "asking again keeps the same single PENDING request"
    );

    room.finish().await;
}
