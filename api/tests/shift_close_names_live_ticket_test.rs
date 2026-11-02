//! `ticketStillServing` names the blocking tickets (design `docs/design/blueprint/queue-tickets.md` §4.7, "The refusal
//! names the blocking tickets", and `xchange-v3-web.md` §4.3 `details`; design PR #5992 with the lane controller's
//! follow-up 1, a typed payload).
//!
//! On both routes that close a shift (the CLOSE `countVerify` and `handoverAccept`), the 409 keeps today's Thai
//! sentence, then `: ` and the ticket numbers, and adds `error.details`, which deserializes into the generated
//! `TicketStillServingDetails`. No live ticket: the close goes through as before. Other refusals carry no `details`.
//!
//! Root cause pinned here: the guard only asked `EXISTS` (`counter_service.rs:207-219`) and the route answered a fixed
//! text (`commands/cash.rs:209-215`), so the staff card could not say which ticket blocked the close (W015 on xc-v3,
//! #564 issuecomment-6577794083).
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own. The harness is `shift_close_live_ticket_route_test.rs`'s, copied so that file stays untouched.
//! `[required]`: `xchange::presentation::dto::v3::ticket_still_serving::{TicketStillServingDetails, LiveTicketDto}`,
//! serde + ts-rs, camelCase: `{tickets: [{number, status, tellerId, tellerName: string | null}]}`.
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

use xchange::presentation::dto::v3::ticket_still_serving::TicketStillServingDetails;

/// A WALK_IN ticket of B01 numbered `W0<seq>` in `status` at `teller`'s counter, called `called_min` minutes ago.
async fn live_ticket(room: &Room, teller: &Who, seq: i32, status: &str, called_min: i32) -> String {
    let served = if status == "SERVING" { "now()" } else { "NULL" };
    room.text(&format!(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, \
             display_number, status, counter_teller_id, issued_at, called_at, served_at, idempotency_key) \
         VALUES (gen_random_uuid(), '{ORG}'::uuid, '{B01}', current_date, 'WALK_IN', {seq}, 'W0{seq}', '{status}', \
             '{}'::uuid, now(), now() - interval '{called_min} minutes', {served}, '{}') RETURNING id::text",
        teller.id,
        unique("live")
    ))
    .await
    .expect("insert the live ticket")
}

async fn name(room: &Room, who: &Who, name: &str) {
    room.text(&format!(
        "UPDATE users SET name = '{name}' WHERE id = '{}'::uuid RETURNING id::text",
        who.id
    ))
    .await
    .expect("name the teller");
}

/// The teller's CLOSE count, submitted; answers the verify route.
async fn close_count(room: &Room, teller: &Who, shift: &str) -> String {
    let lines = count_lines(room).await;
    let close_id = id_of(
        &post(
            room,
            teller,
            "/api/cash-counts",
            json!({ "branch": B01, "kind": "CLOSE", "shiftId": shift, "lines": lines }),
        )
        .await,
        "teller submits CLOSE count",
    );
    format!("/api/cash-counts/{close_id}/verify")
}

fn details_of(a: &Answer) -> TicketStillServingDetails {
    assert_eq!(a.status, StatusCode::CONFLICT, "{}", a.text);
    assert_eq!(
        a.body["error"]["code"],
        json!("ticketStillServing"),
        "{}",
        a.text
    );
    serde_json::from_value(a.body["error"]["details"].clone()).unwrap_or_else(|e| {
        panic!(
            "details deserializes into TicketStillServingDetails ({e}): {}",
            a.text
        )
    })
}

/// N1: the CLOSE verify names W015 (SERVING) and its teller, in the message and in the typed details.
#[actix_web::test]
async fn n1_the_close_verify_names_the_serving_ticket_and_its_teller() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    name(&room, &teller, "Teller One").await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift = open_shift(&room, &teller, &manager).await;
    live_ticket(&room, &teller, 15, "SERVING", 5).await;
    let verify = close_count(&room, &teller, &shift).await;

    let a = post(&room, &manager, &verify, json!({ "branch": B01 })).await;
    let d = details_of(&a);
    assert_eq!(d.tickets.len(), 1, "{}", a.text);
    assert_eq!(d.tickets[0].number, "W015");
    assert_eq!(d.tickets[0].status, "SERVING");
    assert_eq!(d.tickets[0].teller_id, teller.id);
    assert_eq!(d.tickets[0].teller_name.as_deref(), Some("Teller One"));
    assert_eq!(
        a.body["error"]["message"].as_str(),
        Some(format!("{STILL_SERVING_TH}: W015").as_str()),
        "the Thai sentence with the number: {}",
        a.text
    );
    room.finish().await;
}

/// N2: two live tickets (W015 SERVING, called earlier; W016 CALLED) are both listed, by `called_at`.
#[actix_web::test]
async fn n2_two_live_tickets_are_both_listed_in_call_order() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift = open_shift(&room, &teller, &manager).await;
    live_ticket(&room, &teller, 16, "CALLED", 1).await;
    live_ticket(&room, &teller, 15, "SERVING", 9).await;
    let verify = close_count(&room, &teller, &shift).await;

    let a = post(&room, &manager, &verify, json!({ "branch": B01 })).await;
    let d = details_of(&a);
    let got: Vec<(&str, &str)> = d
        .tickets
        .iter()
        .map(|t| (t.number.as_str(), t.status.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![("W015", "SERVING"), ("W016", "CALLED")],
        "{}",
        a.text
    );
    assert_eq!(
        a.body["error"]["message"].as_str(),
        Some(format!("{STILL_SERVING_TH}: W015, W016").as_str()),
        "{}",
        a.text
    );
    room.finish().await;
}

/// N3: no live ticket: the CLOSE verify goes through as before.
#[actix_web::test]
async fn n3_no_live_ticket_the_close_verify_passes_as_before() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift = open_shift(&room, &teller, &manager).await;
    let verify = close_count(&room, &teller, &shift).await;

    let verified = ok(
        &post(&room, &manager, &verify, json!({ "branch": B01 })).await,
        "the verify passes",
    );
    assert_eq!(verified["status"], json!("VERIFIED"));
    room.finish().await;
}

/// N4: the hand-over accept answers the same details for the giver's live ticket.
#[actix_web::test]
async fn n4_the_handover_accept_names_the_givers_ticket() {
    let room = Room::start().await;
    let teller_a = room.person(Role::Teller, Some(B01)).await;
    name(&room, &teller_a, "Teller A").await;
    let teller_b = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift_a = open_shift(&room, &teller_a, &manager).await;
    live_ticket(&room, &teller_a, 21, "CALLED", 2).await;
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

    let a = post(
        &room,
        &teller_b,
        &format!("/api/handovers/{handover_id}/accept"),
        json!({ "branch": B01 }),
    )
    .await;
    let d = details_of(&a);
    assert_eq!(d.tickets.len(), 1, "{}", a.text);
    assert_eq!(d.tickets[0].number, "W021");
    assert_eq!(d.tickets[0].status, "CALLED");
    assert_eq!(d.tickets[0].teller_name.as_deref(), Some("Teller A"));
    room.finish().await;
}

/// N5: the envelope is unchanged for every other refusal: no `details` key (here, a verify of an unknown count).
#[actix_web::test]
async fn n5_other_refusals_carry_no_details() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let a = post(
        &room,
        &manager,
        "/api/cash-counts/00000000-0000-4000-8000-000000000000/verify",
        json!({ "branch": B01 }),
    )
    .await;
    assert_ne!(a.status, StatusCode::OK, "{}", a.text);
    assert!(
        a.body["error"].get("details").is_none(),
        "no details on another refusal: {}",
        a.text
    );
    room.finish().await;
}
