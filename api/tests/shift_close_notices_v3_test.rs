//! Ticket #5096: submitting a CLOSE count writes a `notifications` row with template
//! `close_count_waiting` for the branch's manager, readable through `my_notifications`.

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
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

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
        let schema = unique("closentcv3");
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

        use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
        use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
        let mut tx = pool.begin().await.expect("begin");
        xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Safe(BranchId::new(B01)),
                    currency: xc_kernel::money::CurrencyCode::THB,
                    quantity: xc_kernel::money::ForeignAmount::from_decimal(
                        "1000000.00".parse().expect("an amount"),
                    ),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: unique("fund-thb"),
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

        Self { pool, schema }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let id = xc_testdb::people::seed_person(
            &mut self.pool.acquire().await.expect("acquire"),
            ORG,
            role.as_str(),
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

async fn call(room: &Room, who: &Who, method: Method, uri: &str, body: Option<Value>) -> Value {
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
    assert_eq!(status, StatusCode::OK, "{uri}: {text}");
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[actix_web::test]
async fn submitting_a_close_count_writes_close_count_waiting_readable_through_my_notifications() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let thb1000 = room.denomination("THB", "1000").await;

    let open_res = call(
        &room,
        &teller,
        Method::POST,
        "/api/cash-counts",
        Some(json!({
            "branch": B01,
            "kind": "OPEN",
            "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
        })),
    )
    .await;
    let open_id = open_res["id"].as_str().expect("open count id");

    call(
        &room,
        &manager,
        Method::POST,
        &format!("/api/cash-counts/{open_id}/verify"),
        Some(json!({ "branch": B01 })),
    )
    .await;

    let shift_id: String = sqlx::query_scalar(
        "SELECT id::text FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
    )
    .bind(&teller.id)
    .fetch_one(&room.pool)
    .await
    .expect("open shift");

    let close_res = call(
        &room,
        &teller,
        Method::POST,
        "/api/cash-counts",
        Some(json!({
            "branch": B01,
            "kind": "CLOSE",
            "shiftId": shift_id,
            "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
        })),
    )
    .await;
    let close_id = close_res["id"].as_str().expect("close count id");

    let template_key: String = sqlx::query_scalar(
        "SELECT template_key FROM notifications WHERE recipient_user_id = $1::uuid AND object_id = $2",
    )
    .bind(&manager.id)
    .bind(close_id)
    .fetch_one(&room.pool)
    .await
    .expect("notifications row for manager");
    assert_eq!(template_key, "close_count_waiting");

    let view_res = call(
        &room,
        &manager,
        Method::GET,
        &format!("/api/v3/views/my_notifications?branch={B01}"),
        None,
    )
    .await;
    let items = view_res["data"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "one notification for the branch manager");
    assert_eq!(items[0]["object"]["objectType"], json!("cash_count"));
    assert_eq!(items[0]["object"]["objectId"], json!(close_id));

    room.finish().await;
}
