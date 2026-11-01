//! Triage T4/T10 (2026-11-01 real-staff e2e; design `docs/design/blueprint/staff-e2e-1101-reads.md` §3, ticket
//! #5866): the `cash_counts` view names nobody, shows no difference on a CLOSE count before it is verified, and keeps
//! listing as `WAITING` counts that can no longer be verified.
//!
//! Root cause:
//! - `CashCountRow` (`api/src/presentation/dto/v3/cash.rs:169-191`; query
//!   `api/src/presentation/http/v3/views/cash_counts.rs:75-89`) carries user ids only, and no shift time.
//! - A CLOSE count's `difference` is written only at verify (`xc-cash` `count_service.rs:1251-1270`); submit stores
//!   `NULL` (`count_service.rs:979-986`), so the teller who just counted sees none.
//! - A verify answered `AwaitingFinance` leaves the count unverified (`count_service.rs:1273-1310`), and the view
//!   derives `WAITING` from `verified_by IS NULL AND rejected_by IS NULL` (`cash_counts.rs:83`, `:125-131`), so the
//!   manager keeps seeing verify/reject on a count that is with FINANCE.
//! - A recount makes the older count of the same (shift, kind) unverifiable (`NotNewestCount`,
//!   `count_service.rs:1152-1153`; rule `xc-cash/src/infrastructure/repository.rs:212-245`), but the view still lists
//!   it as `WAITING`.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Pinned here, all through `GET /api/v3/views/cash_counts`:
//! - `countedByName` / `verifiedByName` are `users.name` of `counted_by` / `verified_by`; a blank name (empty or
//!   spaces only) is `null`; an unverified count has `verifiedByName: null`;
//! - `shiftOpenedAt` is `shifts.opened_at` of the count's shift, RFC 3339 with milliseconds and `Z` like `countedAt`;
//!   `null` for a count with no shift (a waiting OPEN count);
//! - `superseded` follows `is_newest_count` in both of its branches (with a shift, and without one by
//!   (branch, counter, kind)): the older row is `true`, the newer `false`; `status=WAITING` lists only the newer; and
//!   the row the view calls superseded is one that `countVerify` refuses with `CashError::NotNewestCount` (told by
//!   its message: the route answers it as `validationFailed`, not as the domain code `notNewestCount`);
//! - a verify answered `AWAITING_FINANCE` makes that row `status: "AWAITING_FINANCE"` with `approvalRequestId` the
//!   PENDING `COUNT_DIFFERENCE` request's id; `status=WAITING` leaves it out and `status=AWAITING_FINANCE` lists it;
//!   once FINANCE approves, the row is `WAITING` again with `approvalRequestId: null`;
//! - a CLOSE count submitted but not verified shows `totals[].difference` = counted − expected while the stored
//!   column is still `NULL`; a SAFE count's stored difference is passed through as stored;
//! - `VERIFIED` and `REJECTED` rows keep their status and their filters; with no filter every row is listed,
//!   superseded ones too.
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
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::domain::error::CashError;
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
                        "t4-fund:{currency}:{}",
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
        let schema = unique("t4rows");
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
            .expect("insert branch");
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
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

    /// A user named `name` holding `role` on `branch` (`None` = organisation-wide).
    async fn named(&self, role: Role, branch: Option<&str>, name: &str) -> Who {
        let rows = vec![xc_testdb::people::RoleRow {
            role_code: role.as_str(),
            branch_id: branch,
            valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            valid_to: None,
        }];
        let id = xc_testdb::people::seed_person(
            &mut self.pool.acquire().await.expect("acquire"),
            ORG,
            name,
            &rows,
        )
        .await
        .expect("insert a user and give them their role");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
            name: name.to_owned(),
        }
    }

    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        self.named(role, branch, &unique("person")).await
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

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// `shifts.opened_at` of `shift`, written as the view writes its times.
    async fn opened_at(&self, shift: &str) -> String {
        let t: DateTime<Utc> =
            sqlx::query_scalar("SELECT opened_at FROM shifts WHERE id::text = $1")
                .bind(shift)
                .fetch_one(&self.pool)
                .await
                .unwrap_or_else(|e| panic!("the shift {shift}: {e}"));
        t.to_rfc3339_opts(SecondsFormat::Millis, true)
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
    name: String,
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

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
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

/// The rows of `cash_counts` at B01 with the query `q` (e.g. `&status=WAITING`), as `who`.
async fn view(room: &Room, who: &Who, q: &str) -> Vec<Value> {
    let a = send(
        room,
        who,
        Method::GET,
        &format!("/api/v3/views/cash_counts?branch={B01}{q}"),
        None,
    )
    .await;
    ok(&a, &format!("cash_counts{q}"))["data"]
        .as_array()
        .unwrap_or_else(|| panic!("cash_counts{q}: `data` is the rows array: {}", a.text))
        .clone()
}

fn ids(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|r| r["id"].as_str().unwrap_or("").to_owned())
        .collect()
}

fn row_of<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or_else(|| panic!("the count {id} is listed: {rows:?}"))
}

fn dec(v: &Value, what: &str) -> Decimal {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a decimal string: {v}"));
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{what} = {s:?}: {e}"))
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

/// A CLOSE count of `shift` holding THB 1000 × 2 and THB 100 × `hundreds`.
async fn close_count_body(room: &Room, shift: &str, hundreds: &str) -> Value {
    json!({
        "branch": B01,
        "kind": "CLOSE",
        "shiftId": shift,
        "lines": [
            { "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "2" },
            { "currency": "THB", "denominationId": room.denomination("THB", "100").await, "qty": hundreds }
        ]
    })
}

async fn submit(room: &Room, who: &Who, body: Value, what: &str) -> String {
    id_of(&post(room, who, "/api/cash-counts", body).await, what)
}

async fn verify(room: &Room, who: &Who, count: &str) -> Answer {
    post(
        room,
        who,
        &format!("/api/cash-counts/{count}/verify"),
        json!({ "branch": B01 }),
    )
    .await
}

/// `teller`'s shift opened over HTTP (THB 2,300 in the drawer); answers (the OPEN count, the shift).
async fn opened_shift(room: &Room, teller: &Who, manager: &Who) -> (String, String) {
    let count = submit(
        room,
        teller,
        open_count_body(room).await,
        "the teller's OPEN count",
    )
    .await;
    ok(
        &verify(room, manager, &count).await,
        "verify the OPEN count",
    );
    let shift = room
        .text(&format!(
            "SELECT id::text FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.id
        ))
        .await
        .expect("the verified OPEN count opened the teller's shift");
    (count, shift)
}

// 1 and 2: who counted and verified, by name; the shift's opening time.
#[actix_web::test]
async fn t4_rows_name_the_counter_and_verifier_and_the_shift_opening() {
    let room = Room::start().await;
    let teller = room.named(Role::Teller, Some(B01), "สมหญิง ใจดี").await;
    let manager = room
        .named(Role::BranchManager, Some(B01), "สมชาย ผู้จัดการ")
        .await;
    let (opened, shift) = opened_shift(&room, &teller, &manager).await;
    let close = submit(
        &room,
        &teller,
        close_count_body(&room, &shift, "3").await,
        "the CLOSE count",
    )
    .await;
    let other = room.named(Role::Teller, Some(B01), "สมศรี รอเปิด").await;
    let waiting_open = submit(
        &room,
        &other,
        open_count_body(&room).await,
        "another teller's OPEN count",
    )
    .await;

    let rows = view(&room, &manager, "").await;
    let opening = room.opened_at(&shift).await;

    let r = row_of(&rows, &opened);
    assert_eq!(r["countedByName"], json!(teller.name), "{r}");
    assert_eq!(r["verifiedByName"], json!(manager.name), "{r}");
    assert_eq!(
        r["shiftOpenedAt"],
        json!(opening),
        "the verified OPEN count names the shift it opened: {r}"
    );

    let r = row_of(&rows, &close);
    assert_eq!(r["countedByName"], json!(teller.name), "{r}");
    assert!(
        r.get("verifiedByName").is_some_and(Value::is_null),
        "an unverified count has verifiedByName: null: {r}"
    );
    assert_eq!(r["shiftOpenedAt"], json!(opening), "{r}");

    let r = row_of(&rows, &waiting_open);
    assert_eq!(r["countedByName"], json!(other.name), "{r}");
    assert!(
        r.get("shiftOpenedAt").is_some_and(Value::is_null),
        "a count with no shift has shiftOpenedAt: null: {r}"
    );
    room.finish().await;
}

// 1: a blank name is null, not "" or spaces.
#[actix_web::test]
async fn t4_a_blank_name_is_null() {
    let room = Room::start().await;
    let teller = room.named(Role::Teller, Some(B01), "").await;
    let manager = room.named(Role::BranchManager, Some(B01), "   ").await;
    let (opened, _) = opened_shift(&room, &teller, &manager).await;

    let rows = view(&room, &manager, "").await;
    let r = row_of(&rows, &opened);
    assert!(
        r.get("countedByName").is_some_and(Value::is_null),
        "an empty name is null: {r}"
    );
    assert!(
        r.get("verifiedByName").is_some_and(Value::is_null),
        "a name of spaces only is null: {r}"
    );
    assert_eq!(
        r["countedBy"],
        json!(teller.id),
        "the id is still there: {r}"
    );
    room.finish().await;
}

// 3: a recount supersedes the older count of the same (shift, kind), in both branches of `is_newest_count`; the
// superseded row is exactly one verify refuses `notNewestCount`.
#[actix_web::test]
async fn t4_a_recount_supersedes_the_older_count_which_verify_refuses() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let (_, shift) = opened_shift(&room, &teller, &manager).await;
    let older_close = submit(
        &room,
        &teller,
        close_count_body(&room, &shift, "3").await,
        "the first CLOSE count",
    )
    .await;
    let newer_close = submit(
        &room,
        &teller,
        close_count_body(&room, &shift, "3").await,
        "the recount",
    )
    .await;
    // no shift: the same teller's OPEN count twice
    let other = room.person(Role::Teller, Some(B01)).await;
    let older_open = submit(&room, &other, open_count_body(&room).await, "first OPEN").await;
    let newer_open = submit(&room, &other, open_count_body(&room).await, "second OPEN").await;

    let all = view(&room, &manager, "").await;
    for (older, newer) in [(&older_close, &newer_close), (&older_open, &newer_open)] {
        assert_eq!(
            row_of(&all, older)["superseded"],
            json!(true),
            "the older count is superseded: {}",
            row_of(&all, older)
        );
        assert_eq!(
            row_of(&all, newer)["superseded"],
            json!(false),
            "the newer count is not: {}",
            row_of(&all, newer)
        );
    }
    assert_eq!(
        ids(&view(&room, &manager, "&status=WAITING").await),
        vec![newer_open.clone(), newer_close.clone()],
        "WAITING lists only the newest count of each (shift, kind), newest first"
    );

    for older in [&older_close, &older_open] {
        let a = verify(&room, &manager, older).await;
        // `countVerify` answers `CashError::NotNewestCount` as its message (the route does not carry the domain
        // code `notNewestCount`), so the refusal is told by that error's text.
        assert_ne!(a.status, StatusCode::OK, "{}", a.text);
        assert_eq!(
            a.body["error"]["message"],
            json!(CashError::NotNewestCount.to_string()),
            "the row the view calls superseded is one verify refuses NotNewestCount: {}",
            a.text
        );
    }
    room.finish().await;
}

// 4: a verify answered AWAITING_FINANCE; then FINANCE approves.
#[actix_web::test]
async fn t4_a_count_with_finance_is_awaiting_finance_until_finance_decides() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let finance = room.person(Role::Finance, Some(B01)).await;
    let (_, shift) = opened_shift(&room, &teller, &manager).await;
    room.exec(&format!(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
         VALUES ('{ORG}'::uuid, '{B01}', 50.00, 'test', now())"
    ))
    .await;
    // the shift expects THB 2,300; this CLOSE count holds THB 2,200: 100 short, over the limit of 50
    let close = submit(
        &room,
        &teller,
        close_count_body(&room, &shift, "2").await,
        "the short CLOSE count",
    )
    .await;
    let a = verify(&room, &manager, &close).await;
    assert_eq!(ok(&a, "verify")["status"], "AWAITING_FINANCE", "{}", a.text);
    let request = room
        .text(&format!(
            "SELECT id::text FROM approval_requests \
             WHERE type = 'COUNT_DIFFERENCE' AND status = 'PENDING' AND object_id = '{close}'"
        ))
        .await
        .expect("one PENDING COUNT_DIFFERENCE request names the count");

    let all = view(&room, &manager, "").await;
    let r = row_of(&all, &close);
    assert_eq!(r["status"], "AWAITING_FINANCE", "{r}");
    assert_eq!(r["approvalRequestId"], json!(request), "{r}");
    assert!(
        !ids(&view(&room, &manager, "&status=WAITING").await).contains(&close),
        "WAITING leaves out a count that is with FINANCE"
    );
    assert_eq!(
        ids(&view(&room, &manager, "&status=AWAITING_FINANCE").await),
        vec![close.clone()],
        "AWAITING_FINANCE lists it"
    );

    let a = post(
        &room,
        &finance,
        &format!("/api/approvals/{request}/decide"),
        json!({ "branch": B01, "id": request, "outcome": "APPROVED", "reason": "t4 up-front test" }),
    )
    .await;
    ok(&a, "FINANCE approves the difference");
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM approval_requests WHERE id::text = '{request}'"
        ))
        .await
        .as_deref(),
        Some("APPROVED")
    );

    let all = view(&room, &manager, "").await;
    let r = row_of(&all, &close);
    assert_eq!(
        r["status"], "WAITING",
        "approved by FINANCE, the manager verifies it again: {r}"
    );
    assert!(
        r.get("approvalRequestId").is_some_and(Value::is_null),
        "no PENDING request any more: {r}"
    );
    assert!(
        ids(&view(&room, &manager, "&status=WAITING").await).contains(&close),
        "back in WAITING"
    );
    assert!(
        view(&room, &manager, "&status=AWAITING_FINANCE")
            .await
            .is_empty(),
        "nothing with FINANCE"
    );
    room.finish().await;
}

// 5: a CLOSE count's difference before verify is derived; a SAFE count's is as stored.
#[actix_web::test]
async fn t4_a_close_count_shows_its_difference_before_verify() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let (_, shift) = opened_shift(&room, &teller, &manager).await;
    let close = submit(
        &room,
        &teller,
        close_count_body(&room, &shift, "2").await,
        "the short CLOSE count",
    )
    .await;
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM cash_count_totals WHERE count_id = '{close}'::uuid AND difference IS NULL"
        ))
        .await,
        1,
        "the stored difference is NULL before verify, so the view must derive it"
    );

    let safe = submit(
        &room,
        &manager,
        json!({
            "branch": B01,
            "kind": "SAFE",
            "lines": [
                { "currency": "USD", "denominationId": room.denomination("USD", "100").await, "qty": "5" }
            ]
        }),
        "the SAFE count",
    )
    .await;
    // a stored difference that is not counted − expected, so only passing it through shows it
    room.exec(&format!(
        "UPDATE cash_count_totals SET difference = 7.00 WHERE count_id = '{safe}'::uuid AND currency = 'USD'"
    ))
    .await;

    let all = view(&room, &manager, "").await;
    let r = row_of(&all, &close);
    let thb = r["totals"]
        .as_array()
        .and_then(|t| t.iter().find(|t| t["currency"] == "THB"))
        .unwrap_or_else(|| panic!("a THB total: {r}"));
    assert_eq!(
        dec(&thb["expected"], "expected"),
        Decimal::from(2300),
        "{thb}"
    );
    assert_eq!(
        dec(&thb["counted"], "counted"),
        Decimal::from(2200),
        "{thb}"
    );
    assert!(
        thb["difference"].is_string(),
        "an unverified CLOSE count shows its difference: {thb}"
    );
    assert_eq!(
        dec(&thb["difference"], "difference"),
        Decimal::from(-100),
        "counted − expected: {thb}"
    );

    let r = row_of(&all, &safe);
    let usd = r["totals"]
        .as_array()
        .and_then(|t| t.iter().find(|t| t["currency"] == "USD"))
        .unwrap_or_else(|| panic!("a USD total: {r}"));
    assert_eq!(
        dec(&usd["difference"], "difference"),
        Decimal::from(7),
        "a stored difference is passed through: {usd}"
    );
    room.finish().await;
}

// 6: VERIFIED and REJECTED keep their status and their filters; no filter lists every row.
#[actix_web::test]
async fn t4_verified_and_rejected_rows_keep_their_status_and_filters() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let (verified, shift) = opened_shift(&room, &teller, &manager).await;
    let rejected = submit(
        &room,
        &teller,
        close_count_body(&room, &shift, "3").await,
        "the CLOSE count to reject",
    )
    .await;
    ok(
        &post(
            &room,
            &manager,
            &format!("/api/cash-counts/{rejected}/reject"),
            json!({ "branch": B01, "reason": "นับใหม่" }),
        )
        .await,
        "reject",
    );
    let recount = submit(
        &room,
        &teller,
        close_count_body(&room, &shift, "3").await,
        "the recount",
    )
    .await;

    let all = view(&room, &manager, "").await;
    assert_eq!(
        ids(&all),
        vec![recount.clone(), rejected.clone(), verified.clone()],
        "no filter lists every row, newest first"
    );
    let r = row_of(&all, &verified);
    assert_eq!(r["status"], "VERIFIED", "{r}");
    assert_eq!(r["superseded"], json!(false), "{r}");
    assert!(
        r.get("approvalRequestId").is_some_and(Value::is_null),
        "{r}"
    );
    let r = row_of(&all, &rejected);
    assert_eq!(r["status"], "REJECTED", "{r}");
    assert_eq!(
        r["superseded"],
        json!(true),
        "the recount is newer by the rule of is_newest_count: {r}"
    );
    let r = row_of(&all, &recount);
    assert_eq!(r["status"], "WAITING", "{r}");
    assert_eq!(r["superseded"], json!(false), "{r}");

    assert_eq!(
        ids(&view(&room, &manager, "&status=VERIFIED").await),
        vec![verified.clone()]
    );
    assert_eq!(
        ids(&view(&room, &manager, "&status=REJECTED").await),
        vec![rejected.clone()],
        "REJECTED lists a rejected row even when it is superseded"
    );
    assert_eq!(
        ids(&view(&room, &manager, "&status=WAITING").await),
        vec![recount.clone()]
    );
    room.finish().await;
}
