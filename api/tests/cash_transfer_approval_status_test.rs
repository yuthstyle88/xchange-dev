//! F5 (eb2849a5 on-screen report, #564 issuecomment-6596339013): a submitted transfer says it is waiting for approval
//! (design `docs/design/blueprint/cash-shift-transfer.md` §4.10, rows T-1..T-7, plus the lane controller's follow-up
//! on PR 6026: T-3/T-4 name "the decider", and T-3a/T-4a pin the within-limit path decided by the source branch's
//! manager, so the tests do not pin only the executive path).
//!
//! The transfer's `status` keeps its state-machine meaning (`DRAFT` until approved; a rejection keeps `DRAFT`, C4).
//! The answers add `approvalRequestId` and `approvalStatus` from the transfer's approval request.
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own. The harness is `v3_cash_commands_test.rs`'s (a schema of its own, branches XC-B01 / XC-B09,
//! each safe funded with USD 500.00 and THB 1,000,000.00). Needs `TEST_DATABASE_URL`; nothing skips.

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
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor as User, Permission, Role, RoleGrants, ScopedRole};
use xc_cash::application::transfer::TransferFlow;
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

// ------------------------------------------------------------ database

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
        let schema = unique("v3cashcmds");
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
            // LG-4 (#929, 1:1): the same USD 500.00 in the ledger's SAFE, which the cutover reads, and THB for
            // the OPEN counts, which now take their cash out of the safe (design §4.4).
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

    /// A user with `role` on `branch` (`None` = organisation-wide): a `users` row, its `user_roles` row and
    /// the `x-test-as` spec that signs them in.
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

/// A signed-in person: their user id and the spec that signs them in.
struct Who {
    id: String,
    spec: String,
}

/// The `access::Actor` of a spec `<user id>|<ROLE>|<branch or *>`.
fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let session_branch = branch.clone().or_else(|| Some(BranchId::new("MAIN")));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(
        User::new(
            id,
            ORG,
            vec![ScopedRole::new(role, branch, from, None)],
            None,
        )
        .with_branch(session_branch),
    )
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

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

enum Key<'a> {
    Fresh,
    This(&'a str),
    Missing,
}

/// One request through the whole route table, as `who` (nobody when `None`).
async fn call(
    room: &Room,
    who: Option<&Who>,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key<'_>,
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
            Key::This(k) => req = req.insert_header(("idempotency-key", k.to_owned())),
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

async fn view(room: &Room, who: &Who, name: &str, query: &str) -> Answer {
    call(
        room,
        Some(who),
        Method::GET,
        &format!("/api/v3/views/{name}?branch={B01}{query}"),
        None,
        Key::Fresh,
    )
    .await
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"].as_str().unwrap_or("").to_owned()
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body.clone()
}

/// The `data` of a view answer.
fn data(a: &Answer, what: &str) -> Value {
    ok(a, what)["data"].clone()
}

/// The id a command answered.
fn id_of(a: &Answer, what: &str) -> String {
    ok(a, what)["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

// ------------------------------------------------------------ counts (G1)

/// A teller's OPEN count body: THB 1000 × 2 + 100 × 3.
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

/// A teller's shift opened over HTTP: the teller submits the OPEN count, a branch manager verifies it.
/// Returns (teller, the count id, the shift id).
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

static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub async fn lg4_fund(
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
                        LG4_FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

// ------------------------------------------------------------ F5 (§4.10)

/// The organisation's transfer limit: USD 300.00 (a fixture row, as `v3_cash_commands_test.rs` sets it).
async fn limit_usd_300(room: &Room, executive: &Who) {
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '300.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(&executive.id)
    .execute(&room.pool)
    .await
    .expect("a transfer policy version");
}

/// A BRANCH transfer of USD 50 × `qty` from B01 to B09, created by `creator`. Returns its id.
async fn created(room: &Room, creator: &Who, qty: &str) -> String {
    let usd50 = room.denomination("USD", "50").await;
    let a = post(
        room,
        creator,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": qty }],
            "note": "to B09"
        }),
    )
    .await;
    id_of(&a, "transferCreate")
}

async fn submit(room: &Room, creator: &Who, id: &str) -> Answer {
    post(
        room,
        creator,
        &format!("/api/cash-transfers/{id}/submit"),
        json!({ "branch": B01 }),
    )
    .await
}

/// The newest approval request of transfer `id`, read from the table the answers must reflect.
async fn request_of(room: &Room, id: &str) -> String {
    room.text(&format!(
        "SELECT approval_request_id::text FROM cash_transfers WHERE id = '{id}'::uuid"
    ))
    .await
    .expect("the transfer has an approval request")
}

async fn decide(room: &Room, decider: &Who, id: &str, outcome: Outcome) {
    let user = user_of(&decider.spec).expect("the decider");
    TransferFlow::pg(room.pool.clone())
        .decide(&user, id, outcome, "decided", Utc::now())
        .await
        .expect("the decider may decide this transfer");
}

/// The `cash_transfers` row of `id`, read by `who`.
async fn row_of(room: &Room, who: &Who, id: &str) -> Value {
    let listed = data(
        &view(room, who, "cash_transfers", "").await,
        "cash_transfers",
    );
    listed
        .as_array()
        .unwrap_or_else(|| panic!("a list of transfers: {listed}"))
        .iter()
        .find(|r| r["id"] == json!(id))
        .cloned()
        .unwrap_or_else(|| panic!("the transfer is listed: {listed}"))
}

/// T-1: submit answers `DRAFT` with the pending request.
#[actix_web::test]
async fn t1_submit_answers_draft_with_a_pending_request() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    limit_usd_300(&room, &executive).await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let id = created(&room, &creator, "2").await;

    let a = submit(&room, &creator, &id).await;
    assert_eq!(a.body["status"], "DRAFT", "{}", a.text);
    assert_eq!(a.body["approvalStatus"], "PENDING", "{}", a.text);
    assert_eq!(
        a.body["approvalRequestId"],
        json!(request_of(&room, &id).await),
        "{}",
        a.text
    );
    room.finish().await;
}

/// T-2: the `cash_transfers` row after submit carries the same request and `PENDING`.
#[actix_web::test]
async fn t2_the_listed_row_says_pending() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    limit_usd_300(&room, &executive).await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let id = created(&room, &creator, "2").await;
    ok(&submit(&room, &creator, &id).await, "transferSubmit");

    let row = row_of(&room, &creator, &id).await;
    assert_eq!(row["status"], "DRAFT", "{row}");
    assert_eq!(row["approvalStatus"], "PENDING", "{row}");
    assert_eq!(
        row["approvalRequestId"],
        json!(request_of(&room, &id).await),
        "{row}"
    );
    room.finish().await;
}

/// T-3a: within the limit (USD 100 ≤ 300), another branch manager of the source approves: `APPROVED` / `APPROVED`.
#[actix_web::test]
async fn t3a_within_the_limit_the_branch_manager_approves() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    limit_usd_300(&room, &executive).await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let id = created(&room, &creator, "2").await;
    ok(&submit(&room, &creator, &id).await, "transferSubmit");
    decide(&room, &manager, &id, Outcome::Approved).await;

    let row = row_of(&room, &creator, &id).await;
    assert_eq!(row["status"], "APPROVED", "{row}");
    assert_eq!(row["approvalStatus"], "APPROVED", "{row}");
    room.finish().await;
}

/// T-3: above the limit (USD 400 > 300), the decider is the Owner (EXECUTIVE): `APPROVED` / `APPROVED`.
#[actix_web::test]
async fn t3_above_the_limit_the_executive_approves() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    limit_usd_300(&room, &executive).await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let id = created(&room, &creator, "8").await;
    ok(&submit(&room, &creator, &id).await, "transferSubmit");
    decide(&room, &executive, &id, Outcome::Approved).await;

    let row = row_of(&room, &creator, &id).await;
    assert_eq!(row["status"], "APPROVED", "{row}");
    assert_eq!(row["approvalStatus"], "APPROVED", "{row}");
    room.finish().await;
}

/// T-4a: within the limit, the branch manager rejects: `DRAFT` (C4) / `REJECTED`.
#[actix_web::test]
async fn t4a_within_the_limit_the_branch_manager_rejects() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    limit_usd_300(&room, &executive).await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let id = created(&room, &creator, "2").await;
    ok(&submit(&room, &creator, &id).await, "transferSubmit");
    decide(&room, &manager, &id, Outcome::Rejected).await;

    let row = row_of(&room, &creator, &id).await;
    assert_eq!(row["status"], "DRAFT", "{row}");
    assert_eq!(row["approvalStatus"], "REJECTED", "{row}");
    room.finish().await;
}

/// T-4: above the limit, the Owner rejects: `DRAFT` / `REJECTED`.
#[actix_web::test]
async fn t4_above_the_limit_the_executive_rejects() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    limit_usd_300(&room, &executive).await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let id = created(&room, &creator, "8").await;
    ok(&submit(&room, &creator, &id).await, "transferSubmit");
    decide(&room, &executive, &id, Outcome::Rejected).await;

    let row = row_of(&room, &creator, &id).await;
    assert_eq!(row["status"], "DRAFT", "{row}");
    assert_eq!(row["approvalStatus"], "REJECTED", "{row}");
    room.finish().await;
}

/// T-5: re-submitting a rejected transfer answers `PENDING` with the new request's id.
#[actix_web::test]
async fn t5_a_resubmit_follows_the_newest_request() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    limit_usd_300(&room, &executive).await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let id = created(&room, &creator, "2").await;
    ok(&submit(&room, &creator, &id).await, "transferSubmit");
    let first = request_of(&room, &id).await;
    decide(&room, &manager, &id, Outcome::Rejected).await;

    let a = submit(&room, &creator, &id).await;
    let second = request_of(&room, &id).await;
    assert_ne!(first, second, "a re-submit raises a new request");
    assert_eq!(a.body["approvalStatus"], "PENDING", "{}", a.text);
    assert_eq!(a.body["approvalRequestId"], json!(second), "{}", a.text);
    let row = row_of(&room, &creator, &id).await;
    assert_eq!(row["approvalStatus"], "PENDING", "{row}");
    assert_eq!(row["approvalRequestId"], json!(second), "{row}");
    room.finish().await;
}

/// T-6: a transfer never submitted carries `null` in both fields, on create's answer and on its row.
#[actix_web::test]
async fn t6_a_transfer_never_submitted_has_no_approval() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let usd50 = room.denomination("USD", "50").await;
    let a = post(
        &room,
        &creator,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
            "note": ""
        }),
    )
    .await;
    let id = id_of(&a, "transferCreate");
    assert_eq!(a.body["approvalStatus"], Value::Null, "{}", a.text);
    assert_eq!(a.body["approvalRequestId"], Value::Null, "{}", a.text);
    assert!(
        a.body
            .as_object()
            .is_some_and(|o| o.contains_key("approvalStatus")),
        "the field is present, as null: {}",
        a.text
    );
    let row = row_of(&room, &creator, &id).await;
    assert_eq!(row["approvalStatus"], Value::Null, "{row}");
    assert_eq!(row["approvalRequestId"], Value::Null, "{row}");
    assert!(
        row.as_object()
            .is_some_and(|o| o.contains_key("approvalStatus")),
        "the field is present, as null: {row}"
    );
    room.finish().await;
}

/// T-7: an OD (`SAFE_TO_DRAWER`) waiting for approval is listed by `cash_transfers?kind=SAFE_TO_DRAWER` with
/// `approvalStatus: "PENDING"`. Restated by the ticket author from the design's T-7, which named `my_shift`:
/// `my_shift` lists only `DISPATCHED` transfers (`views/my_shift.rs:70`), so a pending OD is never there. Both views
/// read the one shared row (`TRANSFER_COLUMNS`), so this row covers the field for both.
#[actix_web::test]
async fn t7_a_pending_od_is_listed_as_pending() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    limit_usd_300(&room, &executive).await;
    let (_teller, _, shift) = opened_shift(&room).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let usd50 = room.denomination("USD", "50").await;
    let a = post(
        &room,
        &manager,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "SAFE_TO_DRAWER",
            "toShiftId": shift,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
            "note": "OD"
        }),
    )
    .await;
    let id = id_of(&a, "transferCreate (SAFE_TO_DRAWER)");
    ok(&submit(&room, &manager, &id).await, "transferSubmit");

    let listed = data(
        &view(&room, &manager, "cash_transfers", "&kind=SAFE_TO_DRAWER").await,
        "cash_transfers",
    );
    let row = listed
        .as_array()
        .unwrap_or_else(|| panic!("a list of transfers: {listed}"))
        .iter()
        .find(|r| r["id"] == json!(id))
        .cloned()
        .unwrap_or_else(|| panic!("the OD is listed: {listed}"));
    assert_eq!(row["kind"], "SAFE_TO_DRAWER", "{row}");
    assert_eq!(row["status"], "DRAFT", "{row}");
    assert_eq!(row["approvalStatus"], "PENDING", "{row}");
    room.finish().await;
}
