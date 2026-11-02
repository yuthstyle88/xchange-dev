//! XC-BE-GAPS-A row 3 (owner ruling R1, 2026-10-27: "ledger-inventory N-1 wins"): a transfer dispatch beyond the
//! source drawer or safe opens a `NEGATIVE_STOCK` approval request, and dispatches below zero once the source
//! branch's BRANCH_MANAGER approves it.
//!
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 3, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Decided, and pinned here:
//! - `docs/design/blueprint/ledger-inventory.md:34` (N-1): a move below zero is refused `stockInsufficient` unless a
//!   BRANCH_MANAGER approves it through AP-1. `cash-shift-transfer.md:276` ("cannot exceed its expected balance") is
//!   superseded by R1.
//! - §5.1 (`ledger-inventory.md:418-425`): the request is opened by the caller that got `stockInsufficient`, here
//!   `POST /api/cash-transfers/{id}/dispatch`. Its `object_id` is the transfer's id exactly as the dispatch
//!   posting's `source_id` (`cash_transfers.id` as text), and its `branch_id` is the dispatcher's branch, so that
//!   branch's manager decides it. The answers are the exchange completion's (S2 #3405,
//!   `s2_completion_negative_stock_test.rs`): 409 `approvalRequired` while pending (one request, whatever the key),
//!   200 on the retry once APPROVED, 409 `stockInsufficient` noting the rejection after a REJECTED (same key, no new
//!   request), and nothing asked within stock.
//! - A refused dispatch leaves the transfer APPROVED with no ledger posting naming it, and the source's balance
//!   unmoved.
//!
//! Not pinned (each as in S2): the request id inside the 409 body; the `proposed_payload` shortfall (row 1); a new
//! key after a rejection.
//!
//! People: B01's BRANCH_MANAGER `bm01` creates, submits and dispatches; a second B01 BRANCH_MANAGER `approver01`
//! approves the transfer (through `TransferFlow::decide`) and decides the `NEGATIVE_STOCK` request over HTTP (the
//! requester may not decide its own, `NegativeStockPolicy::allows_self_decision`). Every safe holds USD 500.00
//! through the ledger. The harness is `transfer_receive_open_shift_and_dispatch_source_test.rs`'s, with a USD 1,000
//! transfer limit and an `Idempotency-Key` the test chooses.
//!
//! **Restated 2026-11-02 by claude/b091f5e0** (ticket author of the no-negative ticket), citing
//! `ledger-inventory.md` §5.1.2 (owner: *"ต้องเติมเงินก่อน ห้ามติดลบ"*), which reverses R1: a dispatch beyond its source
//! drawer or safe answers `409 stockInsufficient` naming the shortfall and opens **no** request; after a top-up of the
//! source the same key dispatches, and nothing goes below zero. The approve-to-negative decisions above are superseded
//! by the tests as they now read.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips. Each test works in a schema
//! of its own, with every migration applied.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// B01's drawer: the OPEN shift of `Cast::teller`.
const SHIFT: &str = "SH-XC-B01";

static FUND: AtomicU64 = AtomicU64::new(0);

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
        let schema = unique("gapsa3");
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
            sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
                .bind(branch)
                .execute(&pool)
                .await
                .expect("the branch's ops_state");
            fund(&pool, CashPlace::Safe(BranchId::new(branch)), "500.00").await;
        }
        Self { pool, schema }
    }

    async fn person(&self, role: Role, branches: &[&str]) -> Who {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let roles: Vec<RoleRow<'_>> = if branches.is_empty() {
            vec![RoleRow {
                role_code: role.as_str(),
                branch_id: None,
                valid_from: from,
                valid_to: None,
            }]
        } else {
            branches
                .iter()
                .map(|b| RoleRow {
                    role_code: role.as_str(),
                    branch_id: Some(*b),
                    valid_from: from,
                    valid_to: None,
                })
                .collect()
        };
        let id = seed_person(
            &mut self.pool.acquire().await.unwrap(),
            ORG,
            &unique(role.as_str()),
            &roles,
        )
        .await
        .expect("insert a user");
        let scope = if branches.is_empty() {
            "*".to_owned()
        } else {
            branches.join(",")
        };
        Who {
            spec: format!("{id}|{}|{scope}", role.as_str()),
            id,
        }
    }

    async fn usd50(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50::numeric",
        )
        .fetch_one(&self.pool)
        .await
        .expect("the USD 50 denomination")
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn open_shift(&self, teller: &Who) {
        self.exec(&format!(
            "INSERT INTO shifts (id, organization_id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('{SHIFT}', '{ORG}'::uuid, '{B01}', 'OPEN', now(), 'test', 'open-{SHIFT}', '{}'::uuid)",
            teller.id
        ))
        .await;
    }

    /// The stored transfer: its status and how many ledger postings name it.
    async fn stored(&self, id: &str) -> (String, i64) {
        let status: String =
            sqlx::query_scalar("SELECT status FROM cash_transfers WHERE id = $1::uuid")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .expect("the stored transfer");
        let postings = self
            .count(&format!(
                "SELECT count(*) FROM ledger_postings WHERE source_id = '{id}'"
            ))
            .await;
        (status, postings)
    }

    /// The `NEGATIVE_STOCK` requests whose `object_id` is `object_id`, as `(id, status, branch_id)` in order.
    async fn requests(&self, object_id: &str) -> Vec<(String, String, Option<String>)> {
        sqlx::query_as(
            "SELECT id::text, status, branch_id FROM approval_requests \
             WHERE type = 'NEGATIVE_STOCK' AND object_id = $1 ORDER BY requested_at, id",
        )
        .bind(object_id)
        .fetch_all(&self.pool)
        .await
        .expect("read approval_requests")
    }

    /// Every `NEGATIVE_STOCK` request in the schema, whatever it names.
    async fn all_negative_stock(&self) -> i64 {
        self.count("SELECT count(*) FROM approval_requests WHERE type = 'NEGATIVE_STOCK'")
            .await
    }

    /// The signed ledger sum of USD in `kind` (`SAFE` of `branch`, or `DRAWER` of `SHIFT`), as `0.00` text.
    async fn usd(&self, place: &str) -> String {
        let filter = match place {
            "drawer" => format!("a.kind = 'DRAWER' AND a.shift_id = '{SHIFT}'"),
            branch => format!("a.kind = 'SAFE' AND a.branch_id = '{branch}'"),
        };
        let sum: rust_decimal::Decimal = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT COALESCE(SUM(e.quantity_signed), 0) FROM ledger_entries e \
             JOIN cash_accounts a ON a.id = e.account_id \
             WHERE {filter} AND trim(a.currency) = 'USD'"
        )))
        .fetch_one(&self.pool)
        .await
        .expect("the signed ledger sum");
        format!("{sum:.2}")
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

/// Puts USD `amount` into `place` through the ledger.
async fn fund(pool: &PgPool, place: CashPlace, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: place,
                currency: xc_kernel::money::CurrencyCode::parse("USD").expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!("gapsa3-fund:{}", FUND.fetch_add(1, Ordering::Relaxed)),
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

// ------------------------------------------------------------ callers

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branches) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let scopes = if branches == "*" {
        vec![ScopedRole::new(role, None, from, None)]
    } else {
        branches
            .split(',')
            .map(|b| ScopedRole::new(role, Some(BranchId::new(b)), from, None))
            .collect()
    };
    let session = (branches != "*")
        .then(|| branches.split(',').next().map(BranchId::new))
        .flatten();
    Some(User::new(id, ORG, scopes, None).with_branch(session))
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

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn message(&self) -> &str {
        self.body["error"]["message"].as_str().unwrap_or("")
    }
}

/// One POST through the whole route table, as `who`, with `key` as the `Idempotency-Key`.
async fn post(room: &Room, who: &Who, uri: &str, body: Value, key: &str) -> Answer {
    let pool = room.pool.clone();
    let spec = who.spec.clone();
    let path = uri.to_owned();
    let key = key.to_owned();
    let handle = actix_web::rt::spawn(async move {
        let state = AppState::new(Arc::new(common::settings(&[])), pool);
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = atest::TestRequest::post()
            .uri(&path)
            .insert_header((header::HOST, HOST))
            .insert_header((AS, spec))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", key))
            .set_payload(body.to_string())
            .to_request();
        let resp = atest::call_service(&app, req).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer { status, text, body }
    });
    match handle.await {
        Ok(a) => a,
        Err(e) => panic!("POST {uri}: the request task failed: {e}"),
    }
}

/// §5.1.2: refused, naming the shortfall, with no request opened.
fn assert_refused(a: &Answer, message: &str, what: &str) {
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "stockInsufficient"),
        "{what}: §5.1.2 — no approval, top up first: {}",
        a.text
    );
    assert_eq!(
        a.message(),
        message,
        "{what}: the message names the shortfall"
    );
}

const SAFE_SHORT: &str = "ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: USD ตู้เซฟ ขาด 100";
const DRAWER_SHORT: &str = "ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: USD ลิ้นชัก ขาด 100";

fn assert_dispatched(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    assert_eq!(a.body["status"], "DISPATCHED", "{what}: {}", a.text);
}

// ------------------------------------------------------------ the people and transfers

struct Cast {
    bm01: Who,
    approver01: Who,
}

async fn cast(room: &Room) -> Cast {
    let executive = room.person(Role::Executive, &[]).await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '1000.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(&executive.id)
    .execute(&room.pool)
    .await
    .expect("a transfer policy version");
    let teller = room.person(Role::Teller, &[B01]).await;
    room.open_shift(&teller).await;
    Cast {
        bm01: room.person(Role::BranchManager, &[B01]).await,
        approver01: room.person(Role::BranchManager, &[B01]).await,
    }
}

/// Creates a B01 transfer of `notes` × USD 50 over HTTP as `bm01`, submits it, and approves it as `approver01`.
async fn approved(room: &Room, c: &Cast, notes: u32, ends: Value) -> String {
    let usd50 = room.usd50().await;
    let mut body = json!({
        "branch": B01,
        "lines": [{ "currency": "USD", "denominationId": usd50, "qty": notes.to_string() }],
        "note": "gapsa3 move"
    });
    for (k, v) in ends.as_object().expect("an object") {
        body[k] = v.clone();
    }
    let a = post(room, &c.bm01, "/api/cash-transfers", body, &unique("key")).await;
    assert_eq!(a.status, StatusCode::OK, "transferCreate: {}", a.text);
    let id = a.body["id"].as_str().expect("an id").to_owned();
    let s = post(
        room,
        &c.bm01,
        &format!("/api/cash-transfers/{id}/submit"),
        json!({ "branch": B01 }),
        &unique("key"),
    )
    .await;
    assert_eq!(s.status, StatusCode::OK, "transferSubmit: {}", s.text);
    TransferFlow::pg(room.pool.clone())
        .decide(
            &user_of(&c.approver01.spec).expect("the approver"),
            &id,
            Outcome::Approved,
            "ok",
            Utc::now(),
        )
        .await
        .expect("a branch manager of the source approves within the limit");
    id
}

/// A BRANCH move B01 → B09 of `notes` × USD 50, APPROVED (B01's safe holds USD 500.00).
async fn branch_move(room: &Room, c: &Cast, notes: u32) -> String {
    approved(
        room,
        c,
        notes,
        json!({ "kind": "BRANCH", "toBranchId": B09 }),
    )
    .await
}

/// `bm01` dispatches transfer `path_id` (the id as the path spells it) with `key`.
async fn dispatch(room: &Room, c: &Cast, path_id: &str, key: &str) -> Answer {
    post(
        room,
        &c.bm01,
        &format!("/api/cash-transfers/{path_id}/dispatch"),
        json!({ "branch": B01 }),
        key,
    )
    .await
}

/// `approver01`, B01's other BRANCH_MANAGER, decides `request_id`; the decision must be accepted.
async fn decide(room: &Room, c: &Cast, request_id: &str, outcome: &str) {
    let a = post(
        room,
        &c.approver01,
        &format!("/api/approvals/{request_id}/decide"),
        json!({ "outcome": outcome, "reason": "gapsa3 up-front test" }),
        &unique("decide"),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "B01's branch manager decides {outcome}: {}",
        a.text
    );
}

/// The transfer is APPROVED, no posting names it, and B01's safe still holds USD 500.00.
async fn assert_not_dispatched(room: &Room, id: &str, what: &str) {
    assert_eq!(
        room.stored(id).await,
        ("APPROVED".to_owned(), 0),
        "{what}: the transfer stays APPROVED and no ledger posting names it"
    );
    assert_eq!(
        room.usd(B01).await,
        "500.00",
        "{what}: B01's safe did not move"
    );
}

// ------------------------------------------------------------ §5.1.2: a shortfall is refused, no request

/// BRANCH USD 600 out of a USD 500 safe: refused naming the safe's shortfall, no request, nothing dispatched.
#[actix_web::test]
async fn gapsa3_beyond_the_safe_is_refused_naming_the_shortfall_and_opens_no_request() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = branch_move(&room, &c, 12).await;

    let a = dispatch(&room, &c, &id, &unique("key")).await;

    assert_refused(&a, SAFE_SHORT, "beyond the safe");
    assert_eq!(room.all_negative_stock().await, 0, "§5.1.2: no request");
    assert_not_dispatched(&room, &id, "refused").await;
    room.finish().await;
}

/// DRAWER_TO_SAFE USD 200 out of a drawer holding USD 100: refused; after USD 100 more in the drawer, the same key
/// dispatches and the drawer ends at zero.
#[actix_web::test]
async fn gapsa3_a_dispatch_beyond_the_drawer_is_refused_and_after_a_top_up_the_same_key_dispatches()
{
    let room = Room::start().await;
    let c = cast(&room).await;
    fund(&room.pool, CashPlace::Drawer(SHIFT.to_owned()), "100.00").await;
    let id = approved(
        &room,
        &c,
        4,
        json!({ "kind": "DRAWER_TO_SAFE", "fromShiftId": SHIFT }),
    )
    .await;
    let key = unique("key");

    let first = dispatch(&room, &c, &id, &key).await;
    assert_refused(&first, DRAWER_SHORT, "beyond the drawer");
    assert_eq!(room.stored(&id).await, ("APPROVED".to_owned(), 0));
    assert_eq!(
        room.usd("drawer").await,
        "100.00",
        "the drawer did not move"
    );

    fund(&room.pool, CashPlace::Drawer(SHIFT.to_owned()), "100.00").await;
    let retry = dispatch(&room, &c, &id, &key).await;
    assert_dispatched(&retry, "the same key after the top-up");
    assert_eq!(room.usd("drawer").await, "0.00", "§5.1.2: never below zero");
    assert_eq!(room.all_negative_stock().await, 0, "no request");
    room.finish().await;
}

// ------------------------------------------------------------ the same refusal, whatever the key

#[actix_web::test]
async fn gapsa3_the_same_key_again_is_refused_the_same_and_opens_no_request() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = branch_move(&room, &c, 12).await;
    let key = unique("key");

    assert_refused(&dispatch(&room, &c, &id, &key).await, SAFE_SHORT, "first");
    assert_refused(
        &dispatch(&room, &c, &id, &key).await,
        SAFE_SHORT,
        "same key",
    );
    assert_eq!(room.all_negative_stock().await, 0, "no request");
    assert_not_dispatched(&room, &id, "same key").await;
    room.finish().await;
}

#[actix_web::test]
async fn gapsa3_a_new_key_is_refused_the_same_and_opens_no_request() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = branch_move(&room, &c, 12).await;

    assert_refused(
        &dispatch(&room, &c, &id, &unique("key")).await,
        SAFE_SHORT,
        "first",
    );
    assert_refused(
        &dispatch(&room, &c, &id, &unique("key")).await,
        SAFE_SHORT,
        "new key",
    );
    assert_eq!(room.all_negative_stock().await, 0, "no request");
    assert_not_dispatched(&room, &id, "new key").await;
    room.finish().await;
}

// ------------------------------------------------------------ after a top-up the same key dispatches

#[actix_web::test]
async fn gapsa3_after_a_top_up_of_the_safe_the_same_key_dispatches_and_the_safe_ends_at_zero() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = branch_move(&room, &c, 12).await;
    let key = unique("key");

    assert_refused(&dispatch(&room, &c, &id, &key).await, SAFE_SHORT, "first");
    fund(&room.pool, CashPlace::Safe(BranchId::new(B01)), "100.00").await;

    let retry = dispatch(&room, &c, &id, &key).await;
    assert_dispatched(&retry, "§5.1.2: the refusal released the key");
    let (status, postings) = room.stored(&id).await;
    assert_eq!(status, "DISPATCHED");
    assert!(postings > 0, "the dispatch posting names the transfer");
    assert_eq!(
        room.usd(B01).await,
        "0.00",
        "600.00 − 600.00, never below zero"
    );
    assert_eq!(room.all_negative_stock().await, 0, "no request");
    room.finish().await;
}

/// The path may spell the id in capitals: still refused the same, and no request under any spelling.
#[actix_web::test]
async fn gapsa3_a_capitalised_path_is_refused_the_same_and_opens_no_request() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = branch_move(&room, &c, 12).await;
    let shouted = id.to_uppercase();
    assert_ne!(shouted, id, "the stored id is lower-case text");

    assert_refused(
        &dispatch(&room, &c, &shouted, &unique("key")).await,
        SAFE_SHORT,
        "capitalised path",
    );
    assert_eq!(
        room.all_negative_stock().await,
        0,
        "no request under any spelling"
    );
    fund(&room.pool, CashPlace::Safe(BranchId::new(B01)), "100.00").await;
    assert_dispatched(
        &dispatch(&room, &c, &shouted, &unique("key")).await,
        "after the top-up",
    );
    assert_eq!(room.usd(B01).await, "0.00");
    room.finish().await;
}

// ------------------------------------------------------------ within stock asks nobody (guard)

#[actix_web::test]
async fn gapsa3_a_dispatch_within_the_safe_opens_no_request() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = branch_move(&room, &c, 2).await;

    let a = dispatch(&room, &c, &id, &unique("key")).await;
    assert_dispatched(&a, "USD 100 is within the safe");
    assert_eq!(room.all_negative_stock().await, 0, "no request is opened");
    assert_eq!(room.usd(B01).await, "400.00");
    room.finish().await;
}
