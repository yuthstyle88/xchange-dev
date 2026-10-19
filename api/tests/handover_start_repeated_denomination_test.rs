//! Ticket #2556: `handoverStart` (`POST /api/handovers`) with a repeated `denominationId`
//! in `lines` answers 400 `validationFailed` and writes nothing, while distinct lines succeed.

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
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);
static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
        let schema = unique("horepdenom");
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
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique(role.as_str()))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
             VALUES ($1::uuid, $2, $3::uuid, $4)",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("give the user a role");
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

fn legacy_of(spec: &str) -> Sessions {
    let mut parts = spec.splitn(3, '|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(id), Some(_), Some("*")) => Sessions {
            owner: Some(Actor::owner(id)),
            staff: None,
        },
        (Some(id), Some(_), Some(branch)) => Sessions {
            owner: None,
            staff: Actor::staff(id, OldRole::Finance, BranchId::new(branch)),
        },
        _ => Sessions {
            owner: None,
            staff: None,
        },
    }
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
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move { legacy_of(&spec) })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let _s = legacy_of(&spec);
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

async fn opened_shift(room: &Room) -> (Who, String, String) {
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let count = id_of(
        &post(
            room,
            &teller,
            "/api/cash-counts",
            open_count_body(room).await,
        )
        .await,
        "the teller submits the OPEN count",
    );
    ok(
        &post(
            room,
            &manager,
            &format!("/api/cash-counts/{count}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "the branch manager verifies the OPEN count",
    );
    let shift = room
        .text(&format!(
            "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.id
        ))
        .await
        .expect("the verified OPEN count opened the teller's shift");
    (teller, count, shift)
}

/// S1: THB 1000 × 1 and THB 1000 × 1 (same `denominationId`) -> 400 `validationFailed`;
/// 0 `handovers` rows; shift still OPEN (main: 500).
#[actix_web::test]
async fn s1_repeated_denomination_id_is_refused_400_validation_failed_and_writes_nothing() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let d1000 = room.denomination("THB", "1000").await;

    let a = post(
        &room,
        &teller,
        "/api/handovers",
        json!({
            "branch": B01,
            "fromShiftId": shift,
            "receiverId": receiver.id,
            "lines": [
                { "currency": "THB", "denominationId": d1000, "qty": "1" },
                { "currency": "THB", "denominationId": d1000, "qty": "1" }
            ]
        }),
    )
    .await;

    assert_eq!(a.status, StatusCode::BAD_REQUEST, "S1 status: {}", a.text);
    assert_eq!(code(&a), "validationFailed", "S1 code: {}", a.text);
    let msg = a.body["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains(&d1000.to_string()),
        "S1 error message must name denomination id {d1000}: {}",
        a.text
    );
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM handovers").await,
        0,
        "S1 must write no handovers row"
    );
    assert_eq!(
        room.text(&format!("SELECT status FROM shifts WHERE id = '{shift}'"))
            .await
            .as_deref(),
        Some("OPEN"),
        "S1 shift must remain OPEN"
    );
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM cash_counts WHERE kind = 'HANDOVER_OUT'")
            .await,
        0,
        "S1 must write no HANDOVER_OUT cash_counts row"
    );
    room.finish().await;
}

/// S2: THB 1000 × 0 and THB 1000 × 2 (same id, one row zero) -> 400 `validationFailed`
/// (still a repeated row).
#[actix_web::test]
async fn s2_repeated_denomination_with_zero_qty_row_is_refused_400_validation_failed() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let d1000 = room.denomination("THB", "1000").await;

    let a = post(
        &room,
        &teller,
        "/api/handovers",
        json!({
            "branch": B01,
            "fromShiftId": shift,
            "receiverId": receiver.id,
            "lines": [
                { "currency": "THB", "denominationId": d1000, "qty": "0" },
                { "currency": "THB", "denominationId": d1000, "qty": "2" }
            ]
        }),
    )
    .await;

    assert_eq!(a.status, StatusCode::BAD_REQUEST, "S2 status: {}", a.text);
    assert_eq!(code(&a), "validationFailed", "S2 code: {}", a.text);
    let msg = a.body["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains(&d1000.to_string()),
        "S2 error message must name denomination id {d1000}: {}",
        a.text
    );
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM handovers").await,
        0,
        "S2 must write no handovers row"
    );
    assert_eq!(
        room.text(&format!("SELECT status FROM shifts WHERE id = '{shift}'"))
            .await
            .as_deref(),
        Some("OPEN"),
        "S2 shift must remain OPEN"
    );
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM cash_counts WHERE kind = 'HANDOVER_OUT'")
            .await,
        0,
        "S2 must write no HANDOVER_OUT cash_counts row"
    );
    room.finish().await;
}

/// S3: THB 1000 × 2 (one line) -> 200 `PENDING` (keep: `started_handover`, `v3_cash_commands_test.rs:977`).
#[actix_web::test]
async fn s3_single_line_succeeds_200_pending() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let d1000 = room.denomination("THB", "1000").await;

    let a = post(
        &room,
        &teller,
        "/api/handovers",
        json!({
            "branch": B01,
            "fromShiftId": shift,
            "receiverId": receiver.id,
            "lines": [
                { "currency": "THB", "denominationId": d1000, "qty": "2" }
            ]
        }),
    )
    .await;

    let handover_id = id_of(&a, "S3 single line");
    assert_eq!(a.body["status"], "PENDING", "{}", a.text);
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM handovers").await,
        1,
        "S3 must write 1 handovers row"
    );
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM handovers WHERE id = '{handover_id}'::uuid"
        ))
        .await
        .as_deref(),
        Some("PENDING"),
        "S3 handover status must be PENDING"
    );
    room.finish().await;
}

/// S4: THB 1000 × 1 and THB 100 × 10 (distinct) -> 200 `PENDING`, both lines stored (keep).
#[actix_web::test]
async fn s4_distinct_denominations_succeed_200_and_store_both_lines() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let d1000 = room.denomination("THB", "1000").await;
    let d100 = room.denomination("THB", "100").await;

    let a = post(
        &room,
        &teller,
        "/api/handovers",
        json!({
            "branch": B01,
            "fromShiftId": shift,
            "receiverId": receiver.id,
            "lines": [
                { "currency": "THB", "denominationId": d1000, "qty": "1" },
                { "currency": "THB", "denominationId": d100, "qty": "10" }
            ]
        }),
    )
    .await;

    let handover_id = id_of(&a, "S4 distinct denominations");
    assert_eq!(a.body["status"], "PENDING", "{}", a.text);
    assert_eq!(
        room.scalar("SELECT COUNT(*) FROM handovers").await,
        1,
        "S4 must write 1 handovers row"
    );
    let count_id = room
        .text(&format!(
            "SELECT count_id::text FROM handovers WHERE id = '{handover_id}'::uuid"
        ))
        .await
        .expect("count_id on handover");
    assert_eq!(
        room.scalar(&format!(
            "SELECT COUNT(*) FROM cash_count_lines WHERE count_id = '{count_id}'::uuid"
        ))
        .await,
        2,
        "S4 must store both count lines"
    );
    room.finish().await;
}
