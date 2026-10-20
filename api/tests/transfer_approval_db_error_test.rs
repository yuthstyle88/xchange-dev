//! A store failure in the approval core during a cash transfer is a server error, not a refusal.
//!
//! Written before the fix by the ticket author (claude/d7aa64), on the xchange lane controller's ruling: a
//! Store/Database error passes through as `AppError::Database` (or `Internal`) and is never flattened into
//! `AppError::Domain(e.to_string())`. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Ruling (lane controller): `ApprovalError::Database` and `ApprovalError::Internal` go to
//! `AppError::Internal`; every other variant keeps `Domain`.
//!
//! The sites, both in `crates/xc-cash/src/application/transfer/transfer_flow.rs`:
//! - `submit_in`: `self.approvals.request(…).map_err(|e| AppError::Domain(e.to_string()))`;
//! - `decide`: `self.approvals.decide(…).map_err(|e| AppError::Domain(e.to_string()))`.
//!
//! `e` is an `ApprovalError`; a database failure is its `Database(String)` variant, displayed
//! `database error: {0}` (`xc-approval/.../approval_service.rs`). Flattened into `Domain`, it reaches
//! `commands/cash.rs::from_cash_error`, which answers every unrecognised `Domain` **400 `validationFailed`
//! with the message**, so the Postgres text goes to the client. `from_cash_error` answers `Internal` /
//! `Database` with `server_error()`: 500 `serverError`, the fixed message `เซิร์ฟเวอร์ขัดข้อง`.
//! `ApprovalError` and its variants are `pub` and xc-cash already depends on xc-approval, so the fix can match
//! `ApprovalError::Database` inside `transfer_flow.rs` alone.
//!
//! How the failure is injected: `TransferFlow::pg` (what the routes build) owns a real `ApprovalService` over
//! `PgApprovalRepository`, so the failure is a real database one, confined to the test's own schema:
//! `CHECK (false) NOT VALID` on the table the approval core writes at that call. Existing rows are not
//! checked; only new rows fail (SQLSTATE 23514).
//! - submit: on `approval_requests`, whose only write on the submit path is `insert_request` inside
//!   `approvals.request`;
//! - decide: on `approval_decisions`, whose only write is `insert_decision` inside `approvals.decide`
//!   (mapped there to `ApprovalError::Database`).
//!
//! Rows:
//! 1. `a_store_failure_opening_the_approval_request_answers_500_server_error` — `POST
//!    /api/cash-transfers/{id}/submit`: status 500, code `serverError`, the body carries none of the
//!    database's words, and no request is stored. **Red on main** (400 `validationFailed` with the text).
//!    The body check does **not** depend on #2029: `from_cash_error`'s 500 is `server_error()`, already generic.
//! 2. `a_store_failure_recording_the_decision_is_a_server_error_not_a_refusal` — `TransferFlow::decide` (it has
//!    no v3 route; the decide route is the approval core's own) returns `AppError::Internal` or
//!    `AppError::Database`, the two `from_cash_error` answers 500, never `Domain`; the transfer stays DRAFT and
//!    its request PENDING. **Red on main** (`Domain("database error: …")`).
//! 3. `a_second_submit_keeps_todays_refusal` — keep-pin: `ApprovalError::AlreadyPending` is a refusal and
//!    stays 400 `validationFailed`. Green on main.
//! 4. `a_decision_above_the_limit_keeps_todays_refusal` — keep-pin: `ApprovalError::PolicyRefused` from
//!    `approvals.decide` stays `AppError::Domain("policy refused: …")`. Green on main.
//! 5. `a_store_failure_writing_the_decision_audit_is_a_server_error_not_a_refusal` — the lane controller's
//!    ruling: `ApprovalError::Internal` also goes to 500. Fault: `CHECK (false) NOT VALID` on `audit_events`
//!    after submit; `approvals.decide` writes it last (`audit_log.append`), after the decision, and maps the
//!    failure to `ApprovalError::Internal("audit log failed: …")`. `TransferFlow::decide` must return
//!    `Internal`/`Database`; the transfer stays DRAFT and its request PENDING (the transaction rolls back).
//!    **Red on main** (`Domain("internal error: …")`), and red under a fix that maps only `Database`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.
//! Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use pos_error::AppError;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

/// Words of a Postgres/sqlx failure that must never reach the client.
const DATABASE_WORDS: [&str; 5] = [
    "database error",
    "relation",
    "constraint",
    "sqlx",
    "numeric",
];

static NEXT: AtomicU64 = AtomicU64::new(0);

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

/// A signed-in person: their user id and the spec that signs them in.
struct Who {
    id: String,
    spec: String,
}

/// Moves `amount` of `currency` from the bank into `branch`'s safe through the ledger.
async fn fund(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
    let mut tx = pool.begin().await.expect("begin");
    LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(BranchId::new(branch)),
                currency: CurrencyCode::parse(currency).expect("a currency"),
                quantity: ForeignAmount::from_decimal(amount.parse().expect("an amount")),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: unique(&format!("fund:{currency}")),
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
        let schema = unique("xfer_db_err");
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
            fund(&pool, branch, "USD", "500.00").await;
        }
        Self { pool, schema }
    }

    /// A user with `role` on `branch` (`None` = organisation-wide).
    async fn person(&self, role: Role, branch: Option<&str>) -> Who {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let id = seed_person(
            &mut self.pool.acquire().await.unwrap(),
            ORG,
            &unique(role.as_str()),
            &[RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: from,
                valid_to: None,
            }],
        )
        .await
        .expect("insert a user");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
        }
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
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

// ------------------------------------------------------------ callers

/// The `access::Actor` of a spec `<user id>|<ROLE>|<branch or *>`.
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

/// The legacy actor the seam carries beside the user; its role does not decide here.
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
}

/// `POST uri` with `body` as `who`, with a fresh `Idempotency-Key`, through the whole route table.
async fn post(room: &Room, who: &Who, uri: &str, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
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

// ------------------------------------------------------------ the transfer

/// A source branch manager (the creator), a second one (the approver) and a USD 300 transfer limit.
struct Cast {
    creator: Who,
    approver: Who,
}

async fn cast(room: &Room) -> Cast {
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '300.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(&executive.id)
    .execute(&room.pool)
    .await
    .expect("a transfer policy version");
    Cast { creator, approver }
}

/// A DRAFT branch transfer B01 → B09 of `notes` USD 50 notes, created over HTTP by the creator.
async fn draft(room: &Room, c: &Cast, notes: u32) -> String {
    let usd50: i64 = sqlx::query_scalar(
        "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50::numeric",
    )
    .fetch_one(&room.pool)
    .await
    .expect("the USD 50 face value");
    let a = post(
        room,
        &c.creator,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": notes.to_string() }],
            "note": "to B09"
        }),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "transferCreate: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("transferCreate answers an id: {}", a.text))
        .to_owned()
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

/// A transfer submitted over HTTP, its approval request PENDING.
async fn submitted(room: &Room, c: &Cast, notes: u32) -> String {
    let id = draft(room, c, notes).await;
    let a = submit(room, c, &id).await;
    assert_eq!(a.status, StatusCode::OK, "transferSubmit: {}", a.text);
    id
}

async fn request_status(room: &Room, id: &str) -> Option<String> {
    room.text(&format!(
        "SELECT r.status FROM approval_requests r \
         JOIN cash_transfers t ON t.approval_request_id = r.id \
         WHERE t.id = '{id}'::uuid"
    ))
    .await
}

async fn transfer_status(room: &Room, id: &str) -> Option<String> {
    room.text(&format!(
        "SELECT status FROM cash_transfers WHERE id = '{id}'::uuid"
    ))
    .await
}

fn leaked(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    DATABASE_WORDS
        .iter()
        .copied()
        .filter(|w| lower.contains(w))
        .collect()
}

// ------------------------------------------------------------ rows

#[actix_web::test]
async fn a_store_failure_opening_the_approval_request_answers_500_server_error() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = draft(&room, &c, 2).await;
    room.exec(
        "ALTER TABLE approval_requests \
         ADD CONSTRAINT transfer_db_error_fault CHECK (false) NOT VALID",
    )
    .await;

    let a = submit(&room, &c, &id).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::INTERNAL_SERVER_ERROR, "serverError"),
        "a database failure inside approvals.request is a server error, never a 400 refusal: body {}",
        a.text
    );
    let found = leaked(&a.text);
    assert!(
        found.is_empty(),
        "the answer carries none of {DATABASE_WORDS:?}; found {found:?} in {}",
        a.text
    );
    assert_eq!(
        room.scalar("SELECT count(*) FROM approval_requests").await,
        0,
        "no approval request is stored"
    );
    assert_eq!(
        room.text(&format!(
            "SELECT approval_request_id::text FROM cash_transfers WHERE id = '{id}'::uuid"
        ))
        .await,
        None,
        "the transfer points at no request"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_store_failure_recording_the_decision_is_a_server_error_not_a_refusal() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = submitted(&room, &c, 2).await; // USD 100 ≤ 300: the source's branch manager decides
    room.exec(
        "ALTER TABLE approval_decisions \
         ADD CONSTRAINT transfer_db_error_fault CHECK (false) NOT VALID",
    )
    .await;

    let approver = user_of(&c.approver.spec).expect("the approver");
    let got = TransferFlow::pg(room.pool.clone())
        .decide(&approver, &id, Outcome::Approved, "ok", Utc::now())
        .await;
    match got {
        Err(AppError::Internal(_)) | Err(AppError::Database(_)) => {}
        Err(other) => panic!(
            "a database failure inside approvals.decide is Internal or Database (from_cash_error's 500), \
             never a refusal; got {other:?}"
        ),
        Ok(t) => panic!("the decision cannot be recorded, yet decide answered {t:?}"),
    }
    assert_eq!(
        transfer_status(&room, &id).await.as_deref(),
        Some("DRAFT"),
        "the transfer is not approved"
    );
    assert_eq!(
        request_status(&room, &id).await.as_deref(),
        Some("PENDING"),
        "the request is still pending"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_store_failure_writing_the_decision_audit_is_a_server_error_not_a_refusal() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = submitted(&room, &c, 2).await; // USD 100 ≤ 300: the source's branch manager decides
    room.exec(
        "ALTER TABLE audit_events \
         ADD CONSTRAINT transfer_db_error_fault CHECK (false) NOT VALID",
    )
    .await;

    let approver = user_of(&c.approver.spec).expect("the approver");
    let got = TransferFlow::pg(room.pool.clone())
        .decide(&approver, &id, Outcome::Approved, "ok", Utc::now())
        .await;
    match got {
        Err(AppError::Internal(_)) | Err(AppError::Database(_)) => {}
        Err(other) => panic!(
            "an audit-log failure inside approvals.decide (ApprovalError::Internal) is Internal or \
             Database (from_cash_error's 500), never a refusal; got {other:?}"
        ),
        Ok(t) => panic!("the decision's audit cannot be written, yet decide answered {t:?}"),
    }
    assert_eq!(
        transfer_status(&room, &id).await.as_deref(),
        Some("DRAFT"),
        "the transfer is not approved"
    );
    assert_eq!(
        request_status(&room, &id).await.as_deref(),
        Some("PENDING"),
        "the request is still pending"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_second_submit_keeps_todays_refusal() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = submitted(&room, &c, 2).await;

    let a = submit(&room, &c, &id).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::BAD_REQUEST, "validationFailed"),
        "ApprovalError::AlreadyPending is a refusal and keeps today's answer: body {}",
        a.text
    );
    assert_eq!(
        room.scalar("SELECT count(*) FROM approval_requests").await,
        1,
        "still one request"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_decision_above_the_limit_keeps_todays_refusal() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = submitted(&room, &c, 8).await; // USD 400 > 300: only an executive decides

    let approver = user_of(&c.approver.spec).expect("the approver");
    let got = TransferFlow::pg(room.pool.clone())
        .decide(&approver, &id, Outcome::Approved, "ok", Utc::now())
        .await;
    match got {
        Err(AppError::Domain(message)) => assert!(
            message.starts_with("policy refused"),
            "the refusal is approvals.decide's PolicyRefused, as today: {message}"
        ),
        Err(other) => panic!("ApprovalError::PolicyRefused stays a Domain refusal; got {other:?}"),
        Ok(t) => panic!("a branch manager above the limit is refused, yet decide answered {t:?}"),
    }
    assert_eq!(
        request_status(&room, &id).await.as_deref(),
        Some("PENDING"),
        "the request is still pending"
    );
    room.finish().await;
}
