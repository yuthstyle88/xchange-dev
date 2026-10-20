//! Tests for #3914: opening/closing a shift through verified cash counts or hand-overs drives
//! the queue counter lifecycle hook (`CounterService` via `ShiftLifecycleHook`) over the HTTP routes.

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
use xc_cash::domain::error::CashError;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

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
        let schema = unique("sc3914");
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

    async fn counters(&self, branch: &str) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT teller_id::text, status FROM queue_counters WHERE branch_id = $1 ORDER BY teller_id",
        )
        .bind(branch)
        .fetch_all(&self.pool)
        .await
        .expect("read queue_counters")
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

async fn count_lines(room: &Room) -> Value {
    json!([
        { "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "2" },
        { "currency": "THB", "denominationId": room.denomination("THB", "100").await, "qty": "3" }
    ])
}

/// C1 & C2: verified OPEN count staffs the teller's counter (`ACTIVE`, `POST /api/queue/open` answers
/// `activeCounters: 1`); verified CLOSE count removes it.
#[actix_web::test]
async fn c1_c2_open_and_close_count_verify_staffs_and_releases_queue_counter() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let lines = count_lines(&room).await;

    let open_id = id_of(
        &post(
            &room,
            &teller,
            "/api/cash-counts",
            json!({ "branch": B01, "kind": "OPEN", "lines": lines }),
        )
        .await,
        "teller submits OPEN count",
    );
    assert!(room.counters(B01).await.is_empty());

    ok(
        &post(
            &room,
            &manager,
            &format!("/api/cash-counts/{open_id}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "manager verifies OPEN count",
    );

    assert_eq!(
        room.counters(B01).await,
        vec![(teller.id.clone(), "ACTIVE".to_owned())],
        "C1: queue_counters has one ACTIVE row for (branch, teller)"
    );

    let queue_open = ok(
        &post(&room, &manager, "/api/queue/open", json!({ "branch": B01 })).await,
        "POST /api/queue/open",
    );
    assert_eq!(queue_open["queueOpen"], json!(true));
    assert_eq!(
        queue_open["activeCounters"],
        json!(1),
        "C1: POST /api/queue/open answers activeCounters: 1"
    );

    let shift_id = room
        .text(&format!(
            "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.id
        ))
        .await
        .expect("teller has an OPEN shift");

    let close_id = id_of(
        &post(
            &room,
            &teller,
            "/api/cash-counts",
            json!({
                "branch": B01,
                "kind": "CLOSE",
                "shiftId": shift_id,
                "lines": lines
            }),
        )
        .await,
        "teller submits CLOSE count",
    );
    ok(
        &post(
            &room,
            &manager,
            &format!("/api/cash-counts/{close_id}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "manager verifies CLOSE count",
    );

    assert!(
        room.counters(B01).await.is_empty(),
        "C2: the teller's queue_counters row is gone after CLOSE verify"
    );

    room.finish().await;
}

/// C3: hand-over accepted from teller A to teller B removes A's row and creates B's row as `ACTIVE`.
#[actix_web::test]
async fn c3_handover_accept_moves_active_counter_from_giver_to_receiver() {
    let room = Room::start().await;
    let teller_a = room.person(Role::Teller, Some(B01)).await;
    let teller_b = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let lines = count_lines(&room).await;

    let open_id = id_of(
        &post(
            &room,
            &teller_a,
            "/api/cash-counts",
            json!({ "branch": B01, "kind": "OPEN", "lines": lines }),
        )
        .await,
        "teller A submits OPEN count",
    );
    ok(
        &post(
            &room,
            &manager,
            &format!("/api/cash-counts/{open_id}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "manager verifies teller A's OPEN count",
    );
    assert_eq!(
        room.counters(B01).await,
        vec![(teller_a.id.clone(), "ACTIVE".to_owned())]
    );

    let shift_a = room
        .text(&format!(
            "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller_a.id
        ))
        .await
        .expect("teller A has an OPEN shift");

    let handover_id = id_of(
        &post(
            &room,
            &teller_a,
            "/api/handovers",
            json!({
                "branch": B01,
                "fromShiftId": shift_a,
                "receiverId": teller_b.id,
                "lines": lines
            }),
        )
        .await,
        "teller A starts handover to teller B",
    );

    ok(
        &post(
            &room,
            &teller_b,
            &format!("/api/handovers/{handover_id}/accept"),
            json!({ "branch": B01 }),
        )
        .await,
        "teller B accepts handover",
    );

    assert_eq!(
        room.counters(B01).await,
        vec![(teller_b.id.clone(), "ACTIVE".to_owned())],
        "C3: A's row is gone and B's row is ACTIVE"
    );

    room.finish().await;
}

/// C4: an OPEN verify refused (verifier = counter, `PersonRules::verifier_is_not_counter`) writes no
/// `queue_counters` row.
#[actix_web::test]
async fn c4_refused_open_verify_writes_no_queue_counter_row() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let lines = count_lines(&room).await;

    let open_id = id_of(
        &post(
            &room,
            &manager,
            "/api/cash-counts",
            json!({ "branch": B01, "kind": "OPEN", "lines": lines }),
        )
        .await,
        "manager submits OPEN count",
    );

    let refused = post(
        &room,
        &manager,
        &format!("/api/cash-counts/{open_id}/verify"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    assert_eq!(
        refused.body["error"]["message"].as_str().unwrap_or(""),
        CashError::SameCounter.to_string()
    );
    assert!(
        room.counters(B01).await.is_empty(),
        "C4: no queue_counters row written when OPEN verify is refused"
    );

    room.finish().await;
}
