//! Ticket #2697: handovers_pending view: a before cursor naming another branch's
//! or receiver's handover slices the page instead of answering empty.

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
use uuid::Uuid;

use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

static LG4_FUND: AtomicU64 = AtomicU64::new(0);

pub async fn lg4_fund(
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
        let schema = unique("handovers_cursor");
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
        for branch in [B01, B09] {
            sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("insert a branch");
            sqlx::query(
                "INSERT INTO ops_state (branch_id, phase) \
                 VALUES ($1, 'OPEN')",
            )
            .bind(branch)
            .execute(&pool)
            .await
            .expect("the branch's safe");
            let safe = || xc_cash::domain::posting::CashPlace::Safe(BranchId::new(branch));
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
        }
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

#[allow(dead_code)]
enum Key {
    Fresh,
    Missing,
}

async fn call(
    room: &Room,
    who: Option<&Who>,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key,
) -> Answer {
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
        .insert_header((header::HOST, HOST));
    if let Some(who) = who {
        req = req.insert_header((AS, who.spec.clone()));
    }
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
        match key {
            Key::Fresh => req = req.insert_header(("idempotency-key", unique("key"))),
            Key::Missing => {}
        }
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn post(room: &Room, who: &Who, uri: &str, body: Value) -> Answer {
    call(room, Some(who), Method::POST, uri, Some(body), Key::Fresh).await
}

async fn view(room: &Room, who: &Who, branch: &str, name: &str, query: &str) -> Answer {
    call(
        room,
        Some(who),
        Method::GET,
        &format!("/api/v3/views/{name}?branch={branch}{query}"),
        None,
        Key::Fresh,
    )
    .await
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body.clone()
}

fn data(a: &Answer, what: &str) -> Value {
    ok(a, what)["data"].clone()
}

fn id_of(a: &Answer, what: &str) -> String {
    ok(a, what)["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

async fn open_count_body_in(room: &Room, branch: &str) -> Value {
    json!({
        "branch": branch,
        "kind": "OPEN",
        "lines": [
            { "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "10" },
            { "currency": "THB", "denominationId": room.denomination("THB", "100").await, "qty": "10" }
        ]
    })
}

async fn opened_shift_in(room: &Room, branch: &str) -> (Who, String, String) {
    let teller = room.person(Role::Teller, Some(branch)).await;
    let manager = room.person(Role::BranchManager, Some(branch)).await;
    let count = id_of(
        &post(
            room,
            &teller,
            "/api/cash-counts",
            open_count_body_in(room, branch).await,
        )
        .await,
        "the teller submits the OPEN count",
    );
    ok(
        &post(
            room,
            &manager,
            &format!("/api/cash-counts/{count}/verify"),
            json!({ "branch": branch }),
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

async fn start_handover(
    room: &Room,
    branch: &str,
    teller: &Who,
    shift: &str,
    receiver: &Who,
) -> String {
    let thb1000 = room.denomination("THB", "1000").await;
    let a = post(
        room,
        teller,
        "/api/handovers",
        json!({
            "branch": branch,
            "fromShiftId": shift,
            "receiverId": receiver.id,
            "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "1" }]
        }),
    )
    .await;
    id_of(&a, "handoverStart")
}

// =================================================================================================
// Behaviour Table Tests (§8)
// =================================================================================================

/// C1: receiver R at B1, `before=<a PENDING handover of branch B2>` -> `rows: []` (main: R's older rows)
#[actix_web::test]
async fn c1_before_cursor_naming_pending_handover_of_other_branch_answers_empty() {
    let room = Room::start().await;
    let receiver_r = room.person(Role::Teller, Some(B01)).await;
    let (teller_b1, _, shift_b1) = opened_shift_in(&room, B01).await;

    // R has two pending handovers at B1: h1 (older), then h2 (newer)
    let _h1 = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_r).await;
    let _h2 = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_r).await;

    // Branch B2 has a pending handover
    let receiver_b2 = room.person(Role::Teller, Some(B09)).await;
    let (teller_b2, _, shift_b2) = opened_shift_in(&room, B09).await;
    let h_b2 = start_handover(&room, B09, &teller_b2, &shift_b2, &receiver_b2).await;

    // Receiver R at B1 queries with before = h_b2 (foreign branch)
    let ans = view(
        &room,
        &receiver_r,
        B01,
        "handovers_pending",
        &format!("&before={h_b2}"),
    )
    .await;
    let rows = data(&ans, "handovers_pending");
    let rows_arr = rows.as_array().expect("array of handovers");

    assert_eq!(
        rows_arr.len(),
        0,
        "C1: before cursor of another branch must answer empty rows: [], got: {rows}"
    );

    room.finish().await;
}

/// C2: receiver R at B1, `before=<a B1 handover to another receiver>` -> `rows: []`
#[actix_web::test]
async fn c2_before_cursor_naming_handover_to_another_receiver_answers_empty() {
    let room = Room::start().await;
    let receiver_r = room.person(Role::Teller, Some(B01)).await;
    let receiver_other = room.person(Role::Teller, Some(B01)).await;
    let (teller_b1, _, shift_b1) = opened_shift_in(&room, B01).await;

    // R has two pending handovers at B1
    let _h1 = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_r).await;
    let _h2 = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_r).await;

    // Another receiver at B1 has a pending handover
    let h_other = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_other).await;

    // Receiver R queries with before = h_other (same branch, different receiver)
    let ans = view(
        &room,
        &receiver_r,
        B01,
        "handovers_pending",
        &format!("&before={h_other}"),
    )
    .await;
    let rows = data(&ans, "handovers_pending");
    let rows_arr = rows.as_array().expect("array of handovers");

    assert_eq!(
        rows_arr.len(),
        0,
        "C2: before cursor to another receiver must answer empty rows: [], got: {rows}"
    );

    room.finish().await;
}

/// C3: `before=<unknown uuid>` -> `rows: []` (kept)
#[actix_web::test]
async fn c3_before_cursor_naming_unknown_uuid_answers_empty() {
    let room = Room::start().await;
    let receiver_r = room.person(Role::Teller, Some(B01)).await;
    let (teller_b1, _, shift_b1) = opened_shift_in(&room, B01).await;

    let _h1 = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_r).await;
    let _h2 = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_r).await;

    let unknown_id = Uuid::new_v4();

    let ans = view(
        &room,
        &receiver_r,
        B01,
        "handovers_pending",
        &format!("&before={unknown_id}"),
    )
    .await;
    let rows = data(&ans, "handovers_pending");
    let rows_arr = rows.as_array().expect("array of handovers");

    assert_eq!(
        rows_arr.len(),
        0,
        "C3: before cursor of unknown uuid must answer empty rows: [], got: {rows}"
    );

    room.finish().await;
}

/// C4: `before=<one of R's own pending handovers>` -> the next page (kept)
#[actix_web::test]
async fn c4_before_cursor_naming_own_pending_handover_answers_next_page() {
    let room = Room::start().await;
    let receiver_r = room.person(Role::Teller, Some(B01)).await;
    let (teller_b1, _, shift_b1) = opened_shift_in(&room, B01).await;

    // R has h1 (older), then h2 (newer)
    let h1 = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_r).await;
    let h2 = start_handover(&room, B01, &teller_b1, &shift_b1, &receiver_r).await;

    // Query before = h2 should return h1 (the next page)
    let ans = view(
        &room,
        &receiver_r,
        B01,
        "handovers_pending",
        &format!("&before={h2}"),
    )
    .await;
    let rows = data(&ans, "handovers_pending");
    let rows_arr = rows.as_array().expect("array of handovers");

    assert_eq!(
        rows_arr.len(),
        1,
        "C4: next page should return exactly 1 row"
    );
    assert_eq!(rows_arr[0]["id"], h1, "C4: next page row should be h1");

    room.finish().await;
}
