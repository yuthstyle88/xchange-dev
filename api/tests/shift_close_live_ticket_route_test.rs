//! #5675 (design `docs/design/blueprint/queue-tickets.md` §4.7, owner ruling A 2026-10-30 13:5x ICT): a shift close
//! is refused `ticketStillServing` (409) while the closing teller holds a CALLED or SERVING ticket, on every route
//! that closes a shift: the CLOSE verify (by a manager, i.e. a close of another teller's shift) and the hand-over
//! accept (the giver's shift). The refusal rolls the whole close back: the shift stays OPEN, the counter stays, the
//! count is not verified, the hand-over stays PENDING. Once the teller ends the ticket, the same verify goes through.
//! W035 on xc-v3 at `0b4400a3` was the symptom: a SERVING ticket outlived its teller's counter and `finish` answered
//! `noOpenShift`.
//!
//! Written before the implementation by the sub-controller (claude/01TxaRJG). Do not edit, delete or weaken; add tests
//! in a file of your own. The harness is `shift_counter_hook_wiring_test.rs`'s, copied so that file stays untouched.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

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
        let schema = unique("sc5675");
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

/// The Thai text of the refusal, as `api/src/presentation/http/v3/commands/cash.rs` answers it.
const STILL_SERVING_TH: &str =
    "ปิดกะไม่ได้ ยังมีคิวที่เรียกหรือกำลังให้บริการค้างอยู่ กรุณาจบคิวนั้นก่อน (เสร็จสิ้น ไม่มีการขาย หรือข้ามคิว)";

/// `teller`'s OPEN count verified by `manager`: answers the OPEN shift's id.
async fn open_shift(room: &Room, teller: &Who, manager: &Who) -> String {
    let lines = count_lines(room).await;
    let open_id = id_of(
        &post(
            room,
            teller,
            "/api/cash-counts",
            json!({ "branch": B01, "kind": "OPEN", "lines": lines }),
        )
        .await,
        "teller submits OPEN count",
    );
    ok(
        &post(
            room,
            manager,
            &format!("/api/cash-counts/{open_id}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "manager verifies OPEN count",
    );
    room.text(&format!(
        "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
        teller.id
    ))
    .await
    .expect("the teller has an OPEN shift")
}

/// A WALK_IN ticket of B01 in `status` (CALLED or SERVING) at `teller`'s counter: answers its id.
async fn live_ticket(room: &Room, teller: &Who, status: &str) -> String {
    let served = if status == "SERVING" { "now()" } else { "NULL" };
    room.text(&format!(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
             display_number, status, counter_teller_id, issued_at, called_at, served_at, idempotency_key) \
         VALUES (gen_random_uuid(), '{ORG}'::uuid, '{B01}', current_date, 'WALK_IN', 35, 'W035', '{status}', \
             '{}'::uuid, now(), now(), {served}, '{}') RETURNING id::text",
        teller.id,
        unique("w035")
    ))
    .await
    .expect("insert the live ticket")
}

async fn shift_status(room: &Room, shift_id: &str) -> String {
    room.text(&format!(
        "SELECT status FROM shifts WHERE id = '{shift_id}'"
    ))
    .await
    .expect("the shift exists")
}

async fn ticket_status(room: &Room, ticket: &str) -> String {
    room.text(&format!(
        "SELECT status FROM queue_tickets WHERE id = '{ticket}'::uuid"
    ))
    .await
    .expect("the ticket exists")
}

fn refused_still_serving(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::CONFLICT, "{what}: {}", a.text);
    assert_eq!(
        a.body["error"]["code"].as_str(),
        Some("ticketStillServing"),
        "{what}: {}",
        a.text
    );
    assert_eq!(
        a.body["error"]["message"].as_str(),
        Some(STILL_SERVING_TH),
        "{what}: {}",
        a.text
    );
}

/// H-1 and H-2: the manager's CLOSE verify of a teller who still serves W035 is refused and closes nothing; the
/// teller releases W035 (#5541) and the manager's second verify of the same count closes the shift.
#[actix_web::test]
async fn h1_h2_a_close_verify_while_the_teller_serves_is_refused_until_the_ticket_ends() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift = open_shift(&room, &teller, &manager).await;
    let w035 = live_ticket(&room, &teller, "SERVING").await;

    let lines = count_lines(&room).await;
    let close_id = id_of(
        &post(
            &room,
            &teller,
            "/api/cash-counts",
            json!({ "branch": B01, "kind": "CLOSE", "shiftId": shift, "lines": lines }),
        )
        .await,
        "teller submits CLOSE count",
    );
    let verify = format!("/api/cash-counts/{close_id}/verify");

    refused_still_serving(
        &post(&room, &manager, &verify, json!({ "branch": B01 })).await,
        "H-1 manager verifies CLOSE count",
    );
    assert_eq!(
        shift_status(&room, &shift).await,
        "OPEN",
        "H-1: the close rolls back"
    );
    assert_eq!(
        room.counters(B01).await,
        vec![(teller.id.clone(), "ACTIVE".to_owned())],
        "H-1: the counter stays"
    );
    assert_eq!(ticket_status(&room, &w035).await, "SERVING");
    assert_eq!(
        room.text(&format!(
            "SELECT verified_at::text FROM cash_counts WHERE id::text = '{close_id}'"
        ))
        .await,
        None,
        "H-1: the CLOSE count is not verified"
    );

    ok(
        &post(
            &room,
            &teller,
            &format!("/api/queue/tickets/{w035}/release"),
            json!({ "branch": B01, "id": w035 }),
        )
        .await,
        "H-2 the teller releases W035",
    );
    let verified = ok(
        &post(&room, &manager, &verify, json!({ "branch": B01 })).await,
        "H-2 manager verifies the same CLOSE count again",
    );
    assert_eq!(verified["status"], json!("VERIFIED"));
    assert_eq!(shift_status(&room, &shift).await, "CLOSED");
    assert!(
        room.counters(B01).await.is_empty(),
        "H-2: the counter is removed"
    );

    room.finish().await;
}

/// H-3: a hand-over accepted while the giver holds a CALLED ticket is refused; the giver keeps the shift and the
/// counter, the receiver gets neither, and the hand-over stays PENDING.
#[actix_web::test]
async fn h3_a_handover_accept_while_the_giver_holds_a_called_ticket_is_refused() {
    let room = Room::start().await;
    let teller_a = room.person(Role::Teller, Some(B01)).await;
    let teller_b = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift_a = open_shift(&room, &teller_a, &manager).await;
    let ticket = live_ticket(&room, &teller_a, "CALLED").await;

    let lines = count_lines(&room).await;
    let handover_id = id_of(
        &post(
            &room,
            &teller_a,
            "/api/handovers",
            json!({ "branch": B01, "fromShiftId": shift_a, "receiverId": teller_b.id, "lines": lines }),
        )
        .await,
        "teller A starts a hand-over to teller B",
    );

    refused_still_serving(
        &post(
            &room,
            &teller_b,
            &format!("/api/handovers/{handover_id}/accept"),
            json!({ "branch": B01 }),
        )
        .await,
        "H-3 teller B accepts",
    );
    assert_eq!(shift_status(&room, &shift_a).await, "OPEN");
    assert_eq!(
        room.counters(B01).await,
        vec![(teller_a.id.clone(), "ACTIVE".to_owned())]
    );
    assert_eq!(
        room.text(&format!(
            "SELECT count(*)::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller_b.id
        ))
        .await
        .as_deref(),
        Some("0"),
        "H-3: the receiver has no OPEN shift"
    );
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM handovers WHERE id::text = '{handover_id}'"
        ))
        .await
        .as_deref(),
        Some("PENDING")
    );
    assert_eq!(ticket_status(&room, &ticket).await, "CALLED");

    room.finish().await;
}
