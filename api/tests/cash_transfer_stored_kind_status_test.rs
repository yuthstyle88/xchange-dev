//! A cash transfer whose stored `kind`, `status` or line `currency` the code cannot read must be refused, never
//! loaded as a default.
//!
//! Written before the fix by the test-writer claude/7c9f3ae3, on the BE lane controller's order. Do not edit,
//! delete or weaken these tests; add your own in another file.
//!
//! **The bug.** `PgTransferRepository`'s row mapper (`api/crates/xc-cash/src/infrastructure/transfer_repository.rs`,
//! `map_transfer`, at main 81e22fe4 `:55-60`) reads
//! - `TransferKind::parse(&row.kind).unwrap_or(TransferKind::Branch)`: an unreadable kind loads as BRANCH, so the
//!   transfer is dispatched, received and reconciled with BRANCH's postings;
//! - `TransferStatus::parse(&row.status).unwrap_or(TransferStatus::Draft)`: an unreadable status loads as DRAFT, so a
//!   transfer whose cash is already in transit can be submitted for approval again (and `on_decided` then sets it
//!   APPROVED unconditionally, `crates/xc-cash/src/application/approval/transfer.rs`, ready for a second dispatch);
//! - `CurrencyCode::parse(..).expect("valid currency")`: an unreadable line currency panics inside the request.
//!
//! **[required] by these tests (the BE controller's ruling):** the reader refuses. `find_by_id` and `find_by_id_in`
//! return `Err(AppError::Internal(_))`; the v3 cash commands that read the transfer through them answer
//! `500 serverError` (`from_cash_error`'s `other` arm), and nothing is written: no state change, no approval
//! request, no ledger posting. Every kind and status the code knows still loads and behaves as today.
//!
//! **How a bad value is stored here.** `cash_transfers.kind` and `.status` carry column CHECKs
//! (`api/migrations/20261012100000_cash.sql:181,186`) and the per-kind `cash_transfers_ends_check`
//! (`api/migrations/20261111160000_cash_transfer_ends.sql:4`), so plain SQL cannot store an unknown value. Each test
//! that needs one drops those CHECKs in its **own** throwaway schema, standing in for the rows a binary rolled back
//! past a newer kind or status would meet (the CHECK is widened by a migration the old binary does not know). The
//! line currency needs no such step: `cash_transfer_lines.currency` only references `currencies(code)`, and a
//! `currencies` row the code's 153-code register does not hold (`XAU`) is accepted by the schema.
//!
//! **Not here:** the `cash_transfers` list view (`src/presentation/http/v3/views/cash_transfers.rs`) reads its own
//! SQL rows and does not go through `map_transfer`; the transfer-limit readers (`find_latest_policy`) are another
//! ticket's.
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
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::domain::transfer::cash_transfer::{CashTransfer, TransferKind, TransferStatus};
use xc_cash::domain::transfer::transfer_repository::TransferRepository;
use xc_cash::infrastructure::PgTransferRepository;
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
/// A kind no `TransferKind` variant reads.
const UNKNOWN_KIND: &str = "BRANCHX";
/// A status no `TransferStatus` variant reads.
const UNKNOWN_STATUS: &str = "IN_TRANSITX";
/// A `currencies` code outside `CurrencyCode`'s register (`xc-kernel/src/money/currency_code.rs`).
const UNKNOWN_CURRENCY: &str = "XAU";

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
        let schema = unique("xferkind");
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
            fund_safe(&pool, branch, "USD", "500.00").await;
        }
        Self { pool, schema }
    }

    /// A user with `role` on `branch` (`None` = organisation-wide).
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

    /// The stored row of a transfer as text: status, kind, dispatched_at set?, received_at set?, approval request.
    async fn stored(&self, id: &str) -> Stored {
        let (status, kind, dispatched, received, approval): (
            String,
            String,
            bool,
            bool,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT status, kind, dispatched_at IS NOT NULL, received_at IS NOT NULL, approval_request_id::text \
             FROM cash_transfers WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("the stored transfer");
        let postings = self
            .count(&format!(
                "SELECT count(*) FROM ledger_postings WHERE source_id = '{id}'"
            ))
            .await;
        let requests = self
            .count(&format!(
                "SELECT count(*) FROM approval_requests WHERE object_id = '{id}'"
            ))
            .await;
        let received_lines = self
            .count(&format!(
                "SELECT count(*) FROM cash_transfer_lines WHERE transfer_id = '{id}'::uuid \
                 AND qty_received IS NOT NULL"
            ))
            .await;
        Stored {
            status,
            kind,
            dispatched,
            received,
            approval,
            postings,
            requests,
            received_lines,
        }
    }

    /// Drops the CHECKs that keep `kind` and `status` to the known values, in this test's schema only.
    async fn allow_unknown_kind_and_status(&self) {
        for check in [
            "cash_transfers_kind_check",
            "cash_transfers_status_check",
            "cash_transfers_ends_check",
        ] {
            self.exec(&format!(
                "ALTER TABLE cash_transfers DROP CONSTRAINT {check}"
            ))
            .await;
        }
    }

    async fn set_kind(&self, id: &str, kind: &str) {
        self.exec(&format!(
            "UPDATE cash_transfers SET kind = '{kind}' WHERE id = '{id}'::uuid"
        ))
        .await;
    }

    async fn set_status(&self, id: &str, status: &str) {
        self.exec(&format!(
            "UPDATE cash_transfers SET status = '{status}' WHERE id = '{id}'::uuid"
        ))
        .await;
    }

    /// Registers `UNKNOWN_CURRENCY` in `currencies` and moves the transfer's lines onto it.
    async fn set_line_currency_unknown(&self, id: &str) {
        self.exec(&format!(
            "INSERT INTO currencies (code, name_th) VALUES ('{UNKNOWN_CURRENCY}', 'ทองคำ') ON CONFLICT DO NOTHING"
        ))
        .await;
        self.exec(&format!(
            "UPDATE cash_transfer_lines SET currency = '{UNKNOWN_CURRENCY}' WHERE transfer_id = '{id}'::uuid"
        ))
        .await;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    status: String,
    kind: String,
    dispatched: bool,
    received: bool,
    approval: Option<String>,
    postings: i64,
    requests: i64,
    received_lines: i64,
}

/// Funds a branch's safe through the ledger, which dispatch checks against.
async fn fund_safe(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(BranchId::new(branch)),
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "xferkind-fund:{currency}:{}",
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

// ------------------------------------------------------------ callers

/// A signed-in person: their user id and the spec that signs them in.
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

/// One POST through the whole route table, as `who`. The request runs on its own task, so a panic in the handler
/// is reported as `Err` (what a client meets: the connection drops with no answer) instead of failing the test
/// with an unrelated message.
async fn post_catching(room: &Room, who: &Who, uri: &str, body: Value) -> Result<Answer, String> {
    let pool = room.pool.clone();
    let spec = who.spec.clone();
    let uri = uri.to_owned();
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
            .uri(&uri)
            .insert_header((header::HOST, HOST))
            .insert_header((AS, spec))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
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
        Ok(a) => Ok(a),
        Err(e) if e.is_panic() => Err(format!("the handler panicked: {e}")),
        Err(e) => Err(format!("the request task failed: {e}")),
    }
}

async fn post(room: &Room, who: &Who, uri: &str, body: Value) -> Answer {
    post_catching(room, who, uri, body)
        .await
        .unwrap_or_else(|e| panic!("POST {uri}: {e}"))
}

fn code(a: &Answer) -> &str {
    a.body["error"]["code"].as_str().unwrap_or("")
}

fn assert_status(a: &Answer, want: &str, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    assert_eq!(a.body["status"], want, "{what}: {}", a.text);
}

/// The ruling: a transfer the reader cannot read is a server fault, `500 serverError`.
fn assert_server_error(a: &Answer, what: &str) {
    assert!(
        a.status == StatusCode::INTERNAL_SERVER_ERROR && code(a) == "serverError",
        "{what}: a stored value the code cannot read must answer 500 serverError, never act on a default; \
         got {} {}",
        a.status,
        a.text
    );
}

// ------------------------------------------------------------ the transfer under test

struct Cast {
    creator: Who,
    approver: Who,
    receiver: Who,
    finance: Who,
}

async fn cast(room: &Room) -> Cast {
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
    Cast {
        creator: room.person(Role::BranchManager, Some(B01)).await,
        approver: room.person(Role::BranchManager, Some(B01)).await,
        receiver: room.person(Role::BranchManager, Some(B09)).await,
        finance: room.person(Role::Finance, None).await,
    }
}

/// A BRANCH transfer B01 → B09 of two USD 50 notes, created over HTTP (DRAFT).
async fn drafted(room: &Room, c: &Cast) -> String {
    let usd50 = room.usd50().await;
    let a = post(
        room,
        &c.creator,
        "/api/cash-transfers",
        json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "2" }],
            "note": "to B09"
        }),
    )
    .await;
    assert_status(&a, "DRAFT", "transferCreate");
    a.body["id"].as_str().expect("an id").to_owned()
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

async fn approve(room: &Room, c: &Cast, id: &str) {
    TransferFlow::pg(room.pool.clone())
        .decide(
            &user_of(&c.approver.spec).expect("the approver"),
            id,
            Outcome::Approved,
            "ok",
            Utc::now(),
        )
        .await
        .expect("a branch manager of the source approves within the limit");
}

/// Created, submitted and approved over the real flow (APPROVED).
async fn approved(room: &Room, c: &Cast) -> String {
    let id = drafted(room, c).await;
    assert_status(&submit(room, c, &id).await, "DRAFT", "transferSubmit");
    approve(room, c, &id).await;
    id
}

async fn dispatch_answer(room: &Room, c: &Cast, id: &str) -> Result<Answer, String> {
    post_catching(
        room,
        &c.creator,
        &format!("/api/cash-transfers/{id}/dispatch"),
        json!({ "branch": B01 }),
    )
    .await
}

async fn receive_answer(room: &Room, c: &Cast, id: &str) -> Answer {
    let usd50 = room.usd50().await;
    post(
        room,
        &c.receiver,
        &format!("/api/cash-transfers/{id}/receive"),
        json!({
            "branch": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qtyReceived": "2" }]
        }),
    )
    .await
}

/// Approved and dispatched over the real flow (DISPATCHED, cash in transit).
async fn dispatched(room: &Room, c: &Cast) -> String {
    let id = approved(room, c).await;
    let a = dispatch_answer(room, c, &id).await.expect("no panic");
    assert_status(&a, "DISPATCHED", "transferDispatch");
    id
}

/// A kind with the ends its `cash_transfers_ends_check` row needs: to branch, from shift, to shift.
type KindEnds = (
    TransferKind,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);

// ------------------------------------------------------------ the repository seam

async fn read_both(
    pool: &PgPool,
    id: &str,
) -> [(&'static str, Result<Option<CashTransfer>, AppError>); 2] {
    let repo = PgTransferRepository::new(pool.clone());
    let on_pool = repo.find_by_id(id).await;
    let mut tx = pool.begin().await.expect("begin");
    let in_tx = repo.find_by_id_in(&mut tx, id).await;
    tx.rollback().await.expect("rollback");
    [("find_by_id", on_pool), ("find_by_id_in", in_tx)]
}

fn assert_refused(what: &str, got: Result<Option<CashTransfer>, AppError>) {
    match got {
        Err(AppError::Internal(_)) => {}
        Ok(Some(t)) => panic!(
            "{what}: the read must refuse a stored value the code cannot read, but it loaded the transfer as \
             kind {:?}, status {:?} (a default stood in for the stored value)",
            t.kind, t.status
        ),
        other => panic!("{what}: expected Err(AppError::Internal(_)), got {other:?}"),
    }
}

/// Runs the repository reads on their own task so the line-currency `expect` shows up as a failure of this test's
/// assertion, not as an unrelated panic.
async fn read_both_catching(
    pool: &PgPool,
    id: &str,
) -> Result<[(&'static str, Result<Option<CashTransfer>, AppError>); 2], String> {
    let pool = pool.clone();
    let id = id.to_owned();
    actix_web::rt::spawn(async move { read_both(&pool, &id).await })
        .await
        .map_err(|e| format!("the read panicked: {e}"))
}

// ============================================================ row K: an unknown stored kind

/// K1 (repository): an unknown stored kind is refused on both reads, never read as BRANCH.
#[actix_web::test]
async fn an_unknown_stored_kind_is_refused_by_both_reads_not_read_as_branch() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = drafted(&room, &c).await;
    room.allow_unknown_kind_and_status().await;
    room.set_kind(&id, UNKNOWN_KIND).await;
    for (what, got) in read_both(&room.pool, &id).await {
        assert_refused(what, got);
    }
    room.finish().await;
}

/// K2 (HTTP, the most harmful path): dispatching an APPROVED transfer whose stored kind is unknown answers 500 and
/// writes nothing. Today it reads as BRANCH, posts BRANCH's safe → in-transit move and answers 200 DISPATCHED.
#[actix_web::test]
async fn dispatch_of_a_transfer_with_an_unknown_stored_kind_answers_500_and_posts_nothing() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = approved(&room, &c).await;
    room.allow_unknown_kind_and_status().await;
    room.set_kind(&id, UNKNOWN_KIND).await;
    let before = room.stored(&id).await;
    let a = dispatch_answer(&room, &c, &id).await.expect("no panic");
    assert_server_error(&a, "transferDispatch on an unknown stored kind");
    assert_eq!(
        room.stored(&id).await,
        before,
        "nothing is written: still APPROVED, no dispatch stamp, no ledger posting"
    );
    assert_eq!(before.status, "APPROVED");
    assert_eq!(before.postings, 0, "no posting before the dispatch");
    room.finish().await;
}

/// K3 (HTTP): submitting a DRAFT transfer whose stored kind is unknown answers 500 and asks no approval.
#[actix_web::test]
async fn submit_of_a_transfer_with_an_unknown_stored_kind_answers_500_and_asks_no_approval() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = drafted(&room, &c).await;
    room.allow_unknown_kind_and_status().await;
    room.set_kind(&id, UNKNOWN_KIND).await;
    let before = room.stored(&id).await;
    let a = submit(&room, &c, &id).await;
    assert_server_error(&a, "transferSubmit on an unknown stored kind");
    let after = room.stored(&id).await;
    assert_eq!(
        after, before,
        "nothing is written: no approval request, no approval_request_id"
    );
    assert_eq!(after.requests, 0);
    room.finish().await;
}

// ============================================================ row S: an unknown stored status

/// S1 (repository): an unknown stored status is refused on both reads, never read as DRAFT.
#[actix_web::test]
async fn an_unknown_stored_status_is_refused_by_both_reads_not_read_as_draft() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = dispatched(&room, &c).await;
    room.allow_unknown_kind_and_status().await;
    room.set_status(&id, UNKNOWN_STATUS).await;
    for (what, got) in read_both(&room.pool, &id).await {
        assert_refused(what, got);
    }
    room.finish().await;
}

/// S2 (HTTP, the most harmful path): a transfer whose cash is already in transit, its stored status now unreadable,
/// must not be submitted for approval again. Today it reads as DRAFT, `transferSubmit` answers 200 and writes a
/// second approval request over the first (`set_approval_request_id` is unconditional), and the approval's
/// `on_decided` would set it APPROVED for a second dispatch.
#[actix_web::test]
async fn submit_of_a_dispatched_transfer_with_an_unknown_status_answers_500_and_asks_no_approval() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = dispatched(&room, &c).await;
    room.allow_unknown_kind_and_status().await;
    room.set_status(&id, UNKNOWN_STATUS).await;
    let before = room.stored(&id).await;
    assert_eq!(before.requests, 1, "the one approval the real flow asked");
    let a = submit(&room, &c, &id).await;
    assert_server_error(&a, "transferSubmit on an unknown stored status");
    assert_eq!(
        room.stored(&id).await,
        before,
        "nothing is written: no second approval request, approval_request_id unchanged"
    );
    room.finish().await;
}

/// S3 (HTTP): dispatch and receive of a transfer whose stored status is unknown answer 500 (a server fault), not
/// the `validationFailed` an illegal step from a DRAFT it is not would give, and write nothing.
#[actix_web::test]
async fn dispatch_and_receive_with_an_unknown_stored_status_answer_500_and_write_nothing() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = dispatched(&room, &c).await;
    room.allow_unknown_kind_and_status().await;
    room.set_status(&id, UNKNOWN_STATUS).await;
    let before = room.stored(&id).await;
    let a = dispatch_answer(&room, &c, &id).await.expect("no panic");
    assert_server_error(&a, "transferDispatch on an unknown stored status");
    let a = receive_answer(&room, &c, &id).await;
    assert_server_error(&a, "transferReceive on an unknown stored status");
    let after = room.stored(&id).await;
    assert_eq!(after, before, "nothing is written");
    assert_eq!(after.received_lines, 0, "no line counted");
    room.finish().await;
}

// ============================================================ row C: an unknown stored line currency

/// C1 (repository): an unknown line currency is refused on both reads, not a panic.
#[actix_web::test]
async fn an_unknown_stored_line_currency_is_refused_by_both_reads_not_a_panic() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = drafted(&room, &c).await;
    room.set_line_currency_unknown(&id).await;
    let reads = read_both_catching(&room.pool, &id)
        .await
        .unwrap_or_else(|e| {
            panic!("an unreadable line currency must be refused with Err(Internal), not panic: {e}")
        });
    for (what, got) in reads {
        assert_refused(what, got);
    }
    room.finish().await;
}

/// C2 (HTTP): dispatching an APPROVED transfer whose line currency is unknown answers 500 serverError (today the
/// handler panics and the client gets no answer) and writes nothing.
#[actix_web::test]
async fn dispatch_with_an_unknown_stored_line_currency_answers_500_not_a_panic() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = approved(&room, &c).await;
    room.set_line_currency_unknown(&id).await;
    let before = room.stored(&id).await;
    let a = dispatch_answer(&room, &c, &id).await.unwrap_or_else(|e| {
        panic!("transferDispatch on an unknown line currency must answer 500 serverError, not panic: {e}")
    });
    assert_server_error(&a, "transferDispatch on an unknown line currency");
    assert_eq!(room.stored(&id).await, before, "nothing is written");
    room.finish().await;
}

// ============================================================ keep: every known kind and status

/// Keep (HTTP): the whole BRANCH walk still answers each status the code knows, every step reading the transfer
/// through the mapper: DRAFT (create, submit), APPROVED (decide, dispatch's locked read), DISPATCHED, RECEIVED,
/// RECONCILED.
#[actix_web::test]
async fn keep_a_branch_transfer_still_walks_every_status_over_http() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = approved(&room, &c).await;
    assert_eq!(room.stored(&id).await.status, "APPROVED");
    let a = dispatch_answer(&room, &c, &id).await.expect("no panic");
    assert_status(&a, "DISPATCHED", "transferDispatch");
    assert_status(
        &receive_answer(&room, &c, &id).await,
        "RECEIVED",
        "transferReceive",
    );
    let a = post(
        &room,
        &c.finance,
        &format!("/api/cash-transfers/{id}/reconcile"),
        json!({ "branch": B01, "note": "checked" }),
    )
    .await;
    assert_status(&a, "RECONCILED", "transferReconcile");
    let s = room.stored(&id).await;
    assert_eq!(
        (s.status.as_str(), s.kind.as_str()),
        ("RECONCILED", "BRANCH")
    );
    assert!(s.dispatched && s.received, "{s:?}");
    assert!(s.postings > 0, "the walk posted: {s:?}");
    room.finish().await;
}

/// Keep (repository): every `TransferKind` and every `TransferStatus` stored with the schema's own values reads back
/// as itself on both reads.
#[actix_web::test]
async fn keep_every_known_kind_and_status_reads_back_as_itself() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let usd50 = room.usd50().await;
    // shifts for the drawer kinds (one CLOSED, so two may stand on B01 beside no OPEN rule)
    for (shift, status) in [("SH-KEEP-1", "OPEN"), ("SH-KEEP-2", "CLOSED")] {
        room.exec(&format!(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('{shift}', '{B01}', '{status}', now(), 'test', '{shift}', '{}'::uuid)",
            c.creator.id
        ))
        .await;
    }
    let kinds: [KindEnds; 5] = [
        (TransferKind::Branch, Some(B09), None, None),
        (TransferKind::DrawerToSafe, None, Some("SH-KEEP-1"), None),
        (TransferKind::SafeToDrawer, None, None, Some("SH-KEEP-2")),
        (TransferKind::ExternalIn, None, None, None),
        (TransferKind::ExternalOut, None, None, None),
    ];
    let statuses = [
        TransferStatus::Draft,
        TransferStatus::Approved,
        TransferStatus::Dispatched,
        TransferStatus::Received,
        TransferStatus::Reconciled,
    ];
    for (i, (kind, to_branch, from_shift, to_shift)) in kinds.into_iter().enumerate() {
        // each kind once, with a status cycling so every status is read at least once
        for status in [statuses[i], statuses[(i + 1) % statuses.len()]] {
            let id: String = sqlx::query_scalar(
                "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, to_branch_id, from_shift_id, \
                    to_shift_id, status, created_by, idempotency_key) \
                 VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8::uuid, $9) RETURNING id::text",
            )
            .bind(ORG)
            .bind(kind.as_str())
            .bind(B01)
            .bind(to_branch)
            .bind(from_shift)
            .bind(to_shift)
            .bind(status.as_str())
            .bind(&c.creator.id)
            .bind(unique("idem"))
            .fetch_one(&room.pool)
            .await
            .unwrap_or_else(|e| panic!("store a {kind:?} {status:?} transfer: {e}"));
            sqlx::query(
                "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, denomination_id, qty_sent) \
                 VALUES ($1::uuid, $2::uuid, 'USD', $3, 2)",
            )
            .bind(&id)
            .bind(ORG)
            .bind(usd50)
            .execute(&room.pool)
            .await
            .expect("a line");
            for (what, got) in read_both(&room.pool, &id).await {
                let t = got
                    .unwrap_or_else(|e| panic!("{what} {kind:?} {status:?}: must load, got {e:?}"))
                    .unwrap_or_else(|| panic!("{what} {kind:?} {status:?}: must be found"));
                assert_eq!(t.kind, kind, "{what}: kind");
                assert_eq!(t.status, status, "{what}: status");
                assert_eq!(t.lines.len(), 1, "{what}: one line");
                assert_eq!(t.lines[0].currency.as_str(), "USD", "{what}: currency");
            }
        }
    }
    room.finish().await;
}
