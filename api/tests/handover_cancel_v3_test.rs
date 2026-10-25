//! Ticket #5107 (BE-3): v3 `handoverCancel` (`POST /api/handovers/{id}/cancel`) and `handoverPending` on
//! `handoverStart` (`POST /api/handovers`).

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
        let schema = unique("ho5107");
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
        .set_payload(body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
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

/// H6: `POST /api/handovers/{id}/cancel` by the giver on a `PENDING` hand-over answers 200 `{id, status: "CANCELLED"}`,
/// refuses a second `POST /api/handovers` while `PENDING` with 409 `handoverPending`, and allows starting a new
/// hand-over once cancelled.
#[actix_web::test]
async fn h6_giver_cancels_pending_handover_over_v3() {
    let room = Room::start().await;
    let giver = room.person(Role::Teller, Some(B01)).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let thb1000 = room.denomination("THB", "1000").await;

    let open_count = id_of(
        &post(
            &room,
            &giver,
            "/api/cash-counts",
            json!({
                "branch": B01,
                "kind": "OPEN",
                "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
            }),
        )
        .await,
        "giver submits OPEN count",
    );
    ok(
        &post(
            &room,
            &manager,
            &format!("/api/cash-counts/{open_count}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "manager verifies OPEN count",
    );
    let shift = room
        .text(&format!(
            "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            giver.id
        ))
        .await
        .expect("giver OPEN shift");

    let start_body = json!({
        "branch": B01,
        "fromShiftId": shift,
        "receiverId": receiver.id,
        "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
    });
    let h1 = id_of(
        &post(&room, &giver, "/api/handovers", start_body.clone()).await,
        "handoverStart",
    );

    let duplicate = post(&room, &giver, "/api/handovers", start_body.clone()).await;
    assert_eq!(
        (duplicate.status, code(&duplicate).as_str()),
        (StatusCode::CONFLICT, "handoverPending"),
        "starting a second hand-over while one is PENDING: {}",
        duplicate.text
    );

    let not_giver = post(
        &room,
        &receiver,
        &format!("/api/handovers/{h1}/cancel"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(
        (not_giver.status, code(&not_giver).as_str()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "non-giver cancelling: {}",
        not_giver.text
    );

    let cancelled = post(
        &room,
        &giver,
        &format!("/api/handovers/{h1}/cancel"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(
        ok(&cancelled, "handoverCancel"),
        json!({ "id": h1, "status": "CANCELLED" })
    );
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM handovers WHERE id = '{h1}'::uuid"
        ))
        .await
        .as_deref(),
        Some("CANCELLED")
    );
    assert_eq!(
        room.text(&format!("SELECT status FROM shifts WHERE id = '{shift}'"))
            .await
            .as_deref(),
        Some("OPEN")
    );

    let h2 = id_of(
        &post(&room, &giver, "/api/handovers", start_body).await,
        "handoverStart after cancel",
    );
    assert_ne!(h1, h2);

    room.finish().await;
}

/// Inserting a second `PENDING` row for the same `from_shift_id` straight into `handovers` is refused by
/// `handovers_one_pending_per_shift` (`23505`).
#[actix_web::test]
async fn handovers_one_pending_per_shift_index_refuses_second_pending_insert() {
    let room = Room::start().await;
    let giver = room.person(Role::Teller, Some(B01)).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let thb1000 = room.denomination("THB", "1000").await;

    let open_count = id_of(
        &post(
            &room,
            &giver,
            "/api/cash-counts",
            json!({
                "branch": B01,
                "kind": "OPEN",
                "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
            }),
        )
        .await,
        "giver submits OPEN count",
    );
    ok(
        &post(
            &room,
            &manager,
            &format!("/api/cash-counts/{open_count}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "manager verifies OPEN count",
    );
    let shift = room
        .text(&format!(
            "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            giver.id
        ))
        .await
        .expect("giver OPEN shift");

    id_of(
        &post(
            &room,
            &giver,
            "/api/handovers",
            json!({
                "branch": B01,
                "fromShiftId": shift,
                "receiverId": receiver.id,
                "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
            }),
        )
        .await,
        "handoverStart",
    );

    let count2: String = sqlx::query_scalar(
        "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, idempotency_key) \
         VALUES ($1::uuid, $2, $3, 'HANDOVER_OUT', $4::uuid, now(), $5) RETURNING id::text",
    )
    .bind(ORG)
    .bind(B01)
    .bind(&shift)
    .bind(&giver.id)
    .bind(unique("seed_count"))
    .fetch_one(&room.pool)
    .await
    .expect("seed second HANDOVER_OUT count");

    let err = sqlx::query(
        "INSERT INTO handovers (organization_id, branch_id, from_shift_id, count_id, receiver_id, status) \
         VALUES ($1::uuid, $2, $3, $4::uuid, $5::uuid, 'PENDING')",
    )
    .bind(ORG)
    .bind(B01)
    .bind(&shift)
    .bind(&count2)
    .bind(&receiver.id)
    .execute(&room.pool)
    .await
    .expect_err("a second PENDING row for the same shift must be refused");

    match err {
        sqlx::Error::Database(db_err) => {
            assert_eq!(db_err.code().as_deref(), Some("23505"), "{db_err}");
            assert_eq!(
                db_err.constraint(),
                Some("handovers_one_pending_per_shift"),
                "{db_err}"
            );
        }
        other => panic!("expected database unique violation, got {other:?}"),
    }

    room.finish().await;
}
