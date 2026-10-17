//! #2063 (caller migration of `Money::minor`, xc-cash transfers), end to end through the real routes: a transfer
//! whose line total (face value × quantity, in `i64` hundredths) or currency sum does not fit is refused
//! `validationFailed` (400) and writes nothing; a large transfer that fits still walks as before.
//!
//! Written before the implementation by the ticket author claude/d7aa64. Do not edit, delete or weaken these tests;
//! add your own in another file. The pure half (every kind's `on_dispatch` / `on_receive`, `is_within_limit`,
//! `sent_minor` / `received_minor`) is `crates/xc-cash/tests/transfer_minor_overflow_test.rs`.
//!
//! **Why 400 and not 500.** The face value is a stored denomination, but the quantity is typed by staff
//! (`qty` / `qtyReceived`, a whole number up to `i32::MAX`, `whole_number` in `http/v3/commands/cash.rs`). Given a
//! denomination of 42,949,673 units or more, staff input alone makes the product overflow, and staff can avoid it by
//! typing a smaller quantity: it is a refused request, not a server fault. The refusal is
//! `TransferError::Validation(_)` → `AppError::Domain` → `from_cash_error` → `validationFailed`, 400 (no new variant).
//!
//! **Choice of this file:** the refusal may come at `transferCreate` (an earlier, stricter check) or at the step that
//! computes the total (submit: the limit check; dispatch; receive). Either way the code is `validationFailed` and
//! nothing is written. Today none of these steps refuses: in debug the handler panics (`attempt to multiply with
//! overflow`), in release the total wraps silently.
//!
//! The harness is `v3_cash_commands_test.rs`'s, trimmed. Needs `TEST_DATABASE_URL`; nothing skips. Each test works
//! in a schema of its own.

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
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::posting::CashPlace;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// The largest face value the transfer repository reads (`Money::parse` caps at 100,000,000 units = 10^10
/// hundredths).
const BIG_FACE: &str = "100000000";
/// 10^10 × 10^9 = 10^19 hundredths: past `i64::MAX` (≈ 9.22 × 10^18).
const OVERFLOWING_QTY: &str = "1000000000";

static NEXT: AtomicU64 = AtomicU64::new(0);
static FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
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
        let schema = unique("cashoverflow");
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
                "INSERT INTO ops_state (branch_id, enabled, phase, safe) \
                 VALUES ($1, true, 'OPEN', jsonb_build_object('USD', 50000::bigint))",
            )
            .bind(branch)
            .execute(&pool)
            .await
            .expect("the branch's safe");
            // USD 100,000,500.00 in each safe's ledger: enough to dispatch one note of the big face value.
            fund(
                &pool,
                CashPlace::Safe(BranchId::new(branch)),
                "USD",
                "100000500.00",
            )
            .await;
        }
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
        }
    }

    /// A USD denomination of `BIG_FACE` units (stored data: only seeds and SQL write face values).
    async fn big_denomination(&self) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('USD', $1::numeric, 'TEST') RETURNING id",
        )
        .bind(BIG_FACE)
        .fetch_one(&self.pool)
        .await
        .expect("insert the big USD denomination")
    }

    async fn policy(&self, usd_limit: &str) {
        let executive: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("policy_setter"))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
             VALUES ('v1', $1::uuid, jsonb_build_object('USD', $2::text), $3::uuid)",
        )
        .bind(ORG)
        .bind(usd_limit)
        .bind(&executive)
        .execute(&self.pool)
        .await
        .expect("a transfer policy version");
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

    async fn status_of(&self, id: &str) -> Option<String> {
        self.text(&format!(
            "SELECT status FROM cash_transfers WHERE id = '{id}'::uuid"
        ))
        .await
    }

    async fn postings_of(&self, id: &str) -> i64 {
        self.scalar(&format!(
            "SELECT count(*) FROM ledger_postings WHERE source_type = 'TRANSFER' AND source_id = '{id}'"
        ))
        .await
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

/// Moves `amount` of `currency` from the bank into `to` through the ledger (`OPENING`), as
/// `v3_cash_commands_test.rs::lg4_fund` does.
async fn fund(pool: &PgPool, to: CashPlace, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to,
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "overflow-fund:{currency}:{}",
                        FUND.fetch_add(1, Ordering::Relaxed)
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
            let s = legacy_of(&spec);
            let user = user_of(&spec);
            Callers {
                owner: s.owner,
                staff: s.staff,
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

fn id_of(a: &Answer, what: &str) -> String {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

fn assert_validation_failed(a: &Answer, what: &str) {
    assert_eq!(code(a), "validationFailed", "{what}: {}", a.text);
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
}

/// The people of one BRANCH transfer B01 → B09.
struct Cast {
    creator: Who,
    receiver: Who,
    executive: Who,
}

async fn cast(room: &Room) -> Cast {
    Cast {
        creator: room.person(Role::BranchManager, Some(B01)).await,
        receiver: room.person(Role::BranchManager, Some(B09)).await,
        executive: room.person(Role::Executive, None).await,
    }
}

/// `transferCreate` of one USD line `qty` × `denomination`: `Ok(id)`, or `Err(answer)` when refused.
async fn create(room: &Room, c: &Cast, denomination: i64, qty: &str) -> Result<String, Answer> {
    let a = post(
        room,
        &c.creator,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": denomination, "qty": qty }],
            "note": "to B09"
        }),
    )
    .await;
    if a.status == StatusCode::OK {
        Ok(id_of(&a, "transferCreate"))
    } else {
        Err(a)
    }
}

async fn submit(room: &Room, c: &Cast, id: &str) -> Answer {
    post(
        room,
        &c.creator,
        &format!("/api/cash-transfers/{id}/submit"),
        json!({ "branch": B01 }),
    )
    .await
}

/// Approved by an executive (with no transfer policy, every transfer is above the limit).
async fn approve(room: &Room, c: &Cast, id: &str) {
    TransferFlow::pg(room.pool.clone())
        .decide(
            &user_of(&c.executive.spec).expect("the executive"),
            id,
            Outcome::Approved,
            "ok",
            Utc::now(),
        )
        .await
        .expect("an executive approves above the limit");
}

async fn dispatch(room: &Room, c: &Cast, id: &str) -> Answer {
    post(
        room,
        &c.creator,
        &format!("/api/cash-transfers/{id}/dispatch"),
        json!({ "branch": B01 }),
    )
    .await
}

async fn receive(room: &Room, c: &Cast, id: &str, denomination: i64, qty: &str) -> Answer {
    post(
        room,
        &c.receiver,
        &format!("/api/cash-transfers/{id}/receive"),
        json!({
            "branch": B09,
            "lines": [{ "currency": "USD", "denominationId": denomination, "qtyReceived": qty }]
        }),
    )
    .await
}

/// A transfer of one big note (10^10 hundredths, fits), created, approved and dispatched.
async fn dispatched_big_note(room: &Room, c: &Cast, big: i64) -> String {
    let id = create(room, c, big, "1")
        .await
        .unwrap_or_else(|a| panic!("one big note is a valid line: {}", a.text));
    id_of(&submit(room, c, &id).await, "transferSubmit");
    approve(room, c, &id).await;
    let a = dispatch(room, c, &id).await;
    assert_eq!(
        a.body["status"], "DISPATCHED",
        "one big note dispatches: {}",
        a.text
    );
    id
}

// ------------------------------------------------------------ the refusals

/// transfer_policy.rs:29 (and :41): with a policy, submit sums the sent lines against the limit. Today the product
/// panics (debug) or wraps to a negative total that reads as *within* the limit (release).
#[actix_web::test]
async fn a_transfer_whose_line_total_overflows_is_refused_at_create_or_submit_and_nothing_is_requested()
 {
    let room = Room::start().await;
    let c = cast(&room).await;
    let big = room.big_denomination().await;
    room.policy("300.00").await;

    match create(&room, &c, big, OVERFLOWING_QTY).await {
        Err(a) => {
            assert_validation_failed(&a, "transferCreate of an overflowing line");
            assert_eq!(
                room.scalar("SELECT count(*) FROM cash_transfers").await,
                0,
                "a refused create writes no transfer"
            );
        }
        Ok(id) => {
            let a = submit(&room, &c, &id).await;
            assert_validation_failed(&a, "transferSubmit of an overflowing line");
            assert_eq!(room.status_of(&id).await.as_deref(), Some("DRAFT"));
            assert_eq!(
                room.text(&format!(
                    "SELECT approval_request_id::text FROM cash_transfers WHERE id = '{id}'::uuid"
                ))
                .await,
                None,
                "a refused submit links no approval request"
            );
            assert_eq!(
                room.scalar(&format!(
                    "SELECT count(*) FROM approval_requests WHERE object_id = '{id}'"
                ))
                .await,
                0,
                "a refused submit opens no approval request"
            );
        }
    }
    room.finish().await;
}

/// kinds/branch.rs:82: with no policy the limit check never runs, so the overflowing line reaches dispatch.
#[actix_web::test]
async fn a_dispatch_whose_line_total_overflows_is_refused_validation_failed_and_nothing_is_posted()
{
    let room = Room::start().await;
    let c = cast(&room).await;
    let big = room.big_denomination().await;

    let id = match create(&room, &c, big, OVERFLOWING_QTY).await {
        Err(a) => {
            // an earlier refusal is allowed (see the file header)
            assert_validation_failed(&a, "transferCreate of an overflowing line");
            room.finish().await;
            return;
        }
        Ok(id) => id,
    };
    id_of(
        &submit(&room, &c, &id).await,
        "transferSubmit with no policy",
    );
    approve(&room, &c, &id).await;
    assert_eq!(room.status_of(&id).await.as_deref(), Some("APPROVED"));

    let a = dispatch(&room, &c, &id).await;
    assert_validation_failed(&a, "transferDispatch of 10^9 notes of 100,000,000");
    assert_eq!(
        room.status_of(&id).await.as_deref(),
        Some("APPROVED"),
        "nothing is dispatched"
    );
    assert_eq!(room.postings_of(&id).await, 0, "nothing is posted");
    room.finish().await;
}

/// kinds/branch.rs:116 and transfer_flow.rs:322/:324: the receiver's counted quantity makes the total overflow.
#[actix_web::test]
async fn a_receipt_whose_counted_total_overflows_is_refused_validation_failed_and_nothing_is_received()
 {
    let room = Room::start().await;
    let c = cast(&room).await;
    let big = room.big_denomination().await;
    let id = dispatched_big_note(&room, &c, big).await;
    assert_eq!(room.postings_of(&id).await, 1, "the dispatch posted");

    let a = receive(&room, &c, &id, big, OVERFLOWING_QTY).await;
    assert_validation_failed(&a, "transferReceive counting 10^9 notes of 100,000,000");
    assert_eq!(
        room.status_of(&id).await.as_deref(),
        Some("DISPATCHED"),
        "nothing is received"
    );
    assert_eq!(
        room.text(&format!(
            "SELECT qty_received::text FROM cash_transfer_lines WHERE transfer_id = '{id}'::uuid"
        ))
        .await,
        None,
        "the counted quantity is not kept"
    );
    assert_eq!(room.postings_of(&id).await, 1, "no receipt is posted");
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM cash_exceptions WHERE data->>'request_id' = '{id}'"
        ))
        .await,
        0,
        "no exception is written"
    );
    room.finish().await;
}

// ------------------------------------------------------------ keep-pins

/// A large transfer that fits walks create → submit → dispatch → receive, posting the exact amount.
#[actix_web::test]
async fn a_large_transfer_that_fits_is_received_with_its_exact_amount() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let big = room.big_denomination().await;
    let id = dispatched_big_note(&room, &c, big).await;

    let a = receive(&room, &c, &id, big, "1").await;
    assert_eq!(a.body["status"], "RECEIVED", "{}", a.text);
    assert_eq!(room.postings_of(&id).await, 2, "dispatch and receipt");
    assert_eq!(
        room.text(&format!(
            "SELECT max(e.quantity_signed)::text FROM ledger_entries e \
             JOIN ledger_postings p ON p.id = e.posting_id \
             WHERE p.source_id = '{id}' AND p.source_key = 'receive'"
        ))
        .await
        .as_deref(),
        Some("100000000.0000"),
        "the destination gains 100,000,000 USD"
    );
    room.finish().await;
}

/// A normal transfer within the policy limit is still within it (the limit check's normal answer is unchanged).
#[actix_web::test]
async fn a_normal_transfer_within_the_limit_is_still_submitted_within_it() {
    let room = Room::start().await;
    let c = cast(&room).await;
    room.policy("300.00").await;
    let usd50: i64 = sqlx::query_scalar(
        "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50",
    )
    .fetch_one(&room.pool)
    .await
    .expect("the USD 50 face value");
    let id = create(&room, &c, usd50, "6")
        .await
        .unwrap_or_else(|a| panic!("6 × 50 USD: {}", a.text));
    id_of(&submit(&room, &c, &id).await, "transferSubmit 300 of 300");
    assert_eq!(
        room.text(&format!(
            "SELECT (r.proposed_payload->>'within_limit') FROM approval_requests r \
             WHERE r.object_id = '{id}'"
        ))
        .await
        .as_deref(),
        Some("true"),
        "300.00 of a 300.00 limit is within it"
    );
    room.finish().await;
}
