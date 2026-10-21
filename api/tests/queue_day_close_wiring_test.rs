//! #4136 W1: the day close over the HTTP routes. Design `docs/design/blueprint/queue-day-close.md` §2 rows 1, 4, 6
//! and 11, §3.3 and §3.5: the CLOSE verify (`countVerify`) and the cut-off (`dayCutoff`) close the branch day,
//! and the routes install `QueueDayHook` (with the booking port) beside `CounterService`.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-9). Do not edit, delete or weaken;
//! add tests in a file of your own. The harness is copied from `shift_counter_hook_wiring_test.rs` (#3914).

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
        let schema = unique("dc4136");
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

// ------------------------------------------------------------ #4136 helpers

/// `teller` submits an OPEN count and `manager` verifies it. Answers the opened shift's id.
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

/// `teller` submits a CLOSE count of what the drawer holds and `manager` verifies it.
async fn close_shift(room: &Room, teller: &Who, manager: &Who, shift_id: &str) {
    let lines = count_lines(room).await;
    let close_id = id_of(
        &post(
            room,
            teller,
            "/api/cash-counts",
            json!({ "branch": B01, "kind": "CLOSE", "shiftId": shift_id, "lines": lines }),
        )
        .await,
        "teller submits CLOSE count",
    );
    ok(
        &post(
            room,
            manager,
            &format!("/api/cash-counts/{close_id}/verify"),
            json!({ "branch": B01 }),
        )
        .await,
        "manager verifies CLOSE count",
    );
}

async fn cut_off(room: &Room, manager: &Who) {
    let answer = ok(
        &post(
            room,
            manager,
            "/api/branch-day/cutoff",
            json!({ "branch": B01 }),
        )
        .await,
        "manager cuts off",
    );
    assert_eq!(answer["phase"], json!("CUT_OFF"));
}

async fn open_queue(room: &Room, manager: &Who) {
    ok(
        &post(room, manager, "/api/queue/open", json!({ "branch": B01 })).await,
        "POST /api/queue/open",
    );
}

/// A walk-in ticket issued in-process (the issuance route is not under test). Answers its id.
async fn walk_in(room: &Room, key: &str) -> String {
    let branch_tx = xc_kernel::branch_tx::BranchTx::new(pos_infra::transactions::Transactor::new(
        room.pool.clone(),
    ));
    xc_queue::application::issue::ticket_issuer::TicketIssuer::new(
        branch_tx,
        Arc::new(xc_queue::infrastructure::repository::PgQueueRepository::new(room.pool.clone())),
    )
    .issue(
        &BranchId::new(B01),
        xc_queue::domain::ticket::issue_source::IssueSource::Desk,
        key,
        Utc::now(),
    )
    .await
    .unwrap_or_else(|e| panic!("issue {key}: {e:?}"))
    .id
}

/// `status reason` of a ticket as stored (`-` for no reason).
async fn ticket(room: &Room, id: &str) -> String {
    room.text(&format!(
        "SELECT status || ' ' || COALESCE(ended_reason, '-') FROM queue_tickets WHERE id = '{id}'::uuid"
    ))
    .await
    .expect("the ticket")
}

// ------------------------------------------------------------ #4136 rows

/// Row 1 (and row 12 on the way): after cut-off, the CLOSE verify of the last OPEN shift cancels the queue's
/// WAITING tickets with `dayClosed`. The cut-off itself, with that shift still OPEN, cancels nothing.
#[actix_web::test]
async fn w1_row1_the_last_close_verify_after_cut_off_cancels_waiting_tickets() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift = open_shift(&room, &teller, &manager).await;
    open_queue(&room, &manager).await;
    let first = walk_in(&room, "w1-r1-a").await;
    let second = walk_in(&room, "w1-r1-b").await;

    cut_off(&room, &manager).await;
    assert_eq!(
        ticket(&room, &first).await,
        "WAITING -",
        "a cut-off with a shift OPEN closes nothing"
    );

    close_shift(&room, &teller, &manager, &shift).await;

    assert_eq!(ticket(&room, &first).await, "CANCELLED dayClosed");
    assert_eq!(ticket(&room, &second).await, "CANCELLED dayClosed");
    room.finish().await;
}

/// Row 2 (over HTTP): a CLOSE verify after cut-off while another teller's shift is OPEN cancels nothing.
#[actix_web::test]
async fn w1_row2_a_close_verify_with_another_shift_open_cancels_nothing() {
    let room = Room::start().await;
    let teller_a = room.person(Role::Teller, Some(B01)).await;
    let teller_b = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift_a = open_shift(&room, &teller_a, &manager).await;
    open_shift(&room, &teller_b, &manager).await;
    open_queue(&room, &manager).await;
    let waiting = walk_in(&room, "w1-r2").await;

    cut_off(&room, &manager).await;
    close_shift(&room, &teller_a, &manager, &shift_a).await;

    assert_eq!(ticket(&room, &waiting).await, "WAITING -");
    room.finish().await;
}

/// Row 11: every CLOSE was verified while the day was OPEN, so the cut-off route closes the day and cancels the
/// WAITING tickets.
#[actix_web::test]
async fn w1_row11_a_cut_off_with_no_open_shift_cancels_waiting_tickets() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift = open_shift(&room, &teller, &manager).await;
    open_queue(&room, &manager).await;
    let waiting = walk_in(&room, "w1-r11").await;

    close_shift(&room, &teller, &manager, &shift).await;
    assert_eq!(
        ticket(&room, &waiting).await,
        "WAITING -",
        "the day is still OPEN at the verify"
    );

    cut_off(&room, &manager).await;

    assert_eq!(ticket(&room, &waiting).await, "CANCELLED dayClosed");
    room.finish().await;
}

/// Row 4: a hand-over accepted after cut-off by the branch's last teller closes the giver's shift but not the
/// day, because the receiver's shift opens in the same transaction and the hand-over never asks.
#[actix_web::test]
async fn w1_row4_a_handover_after_cut_off_cancels_nothing() {
    let room = Room::start().await;
    let teller_a = room.person(Role::Teller, Some(B01)).await;
    let teller_b = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift_a = open_shift(&room, &teller_a, &manager).await;
    open_queue(&room, &manager).await;
    let waiting = walk_in(&room, "w1-r4").await;
    cut_off(&room, &manager).await;

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
    ok(
        &post(
            &room,
            &teller_b,
            &format!("/api/handovers/{handover_id}/accept"),
            json!({ "branch": B01 }),
        )
        .await,
        "teller B accepts the hand-over",
    );

    assert_eq!(ticket(&room, &waiting).await, "WAITING -");
    room.finish().await;
}

/// Row 6: the route's hook carries the booking port — a CHECKED_IN booking whose ticket is WAITING becomes
/// CANCELLED with `dayClosed` when the last CLOSE verify closes the day.
#[actix_web::test]
async fn w1_row6_a_checked_in_booking_is_cancelled_at_day_close() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let shift = open_shift(&room, &teller, &manager).await;
    open_queue(&room, &manager).await;

    for sql in [
        format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('CU-W1', '{ORG}'::uuid, 'CU-W1', 'Booked', 'LIVE', now())"
        ),
        format!(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             VALUES (gen_random_uuid(), '{ORG}'::uuid, '{B01}', current_date, now(), now() + interval '30 minutes', 4, 1, 'OPEN')"
        ),
        format!(
            "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
                 requested_currency, requested_quantity, source_channel, status, qr_token_hash, checked_in_at, created_by) \
             SELECT 'BK-W1', s.organization_id, s.branch_id, 'B-W1', 'CU-W1', s.id, 'BUY', 'USD', 100, 'DESK', \
                 'CHECKED_IN', convert_to('qr-W1', 'UTF8'), now(), 'test' \
             FROM booking_slots s WHERE s.branch_id = '{B01}' LIMIT 1"
        ),
    ] {
        sqlx::query(AssertSqlSafe(sql.clone()))
            .execute(&room.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    let branch_tx = xc_kernel::branch_tx::BranchTx::new(pos_infra::transactions::Transactor::new(
        room.pool.clone(),
    ));
    let booked = xc_queue::application::issue::ticket_issuer::TicketIssuer::new(
        branch_tx,
        Arc::new(xc_queue::infrastructure::repository::PgQueueRepository::new(room.pool.clone())),
    )
    .issue(
        &BranchId::new(B01),
        xc_queue::domain::ticket::issue_source::IssueSource::Booking {
            booking_id: "BK-W1".to_owned(),
        },
        "booking:BK-W1",
        Utc::now(),
    )
    .await
    .expect("the booking's ticket")
    .id;

    cut_off(&room, &manager).await;
    close_shift(&room, &teller, &manager, &shift).await;

    assert_eq!(ticket(&room, &booked).await, "CANCELLED dayClosed");
    assert_eq!(
        room.text(
            "SELECT status || ' ' || COALESCE(ended_reason, '-') FROM bookings WHERE id = 'BK-W1'"
        )
        .await
        .as_deref(),
        Some("CANCELLED dayClosed"),
        "the booking port was told"
    );
    room.finish().await;
}
