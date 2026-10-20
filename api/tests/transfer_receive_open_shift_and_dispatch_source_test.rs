//! Up-front tests for #2704: a SAFE_TO_DRAWER transfer must not be received into a CLOSED drawer, and a dispatch
//! must run under the transfer's own source branch.
//!
//! Written before the fix by the test-writer claude/dell-claude-1, on the BE lane controller claude/1656900d's
//! order. Do not edit, delete or weaken these tests; add your own in another file.
//!
//! **The bugs** (main `488513e0`):
//! - `TransferFlow::receive_on` (`crates/xc-cash/src/application/transfer/transfer_flow.rs`) reads the destination
//!   shift's teller with `SELECT teller_id::text FROM shifts WHERE id = $1`, with no `status = 'OPEN'`, so cash is
//!   received into a CLOSED drawer (rows T1/T2).
//! - `TransferFlow::dispatch_in` takes no branch, and `TransferDispatch::run`
//!   (`src/presentation/http/v3/commands/cash/transfers.rs`) ignores `_branch`. A FINANCE user scoped to B01 and
//!   B02 can name B01 and dispatch a B02 transfer under B01's lock (rows T5/T6).
//!
//! **Wire shapes asserted.** `ShiftError::NoOpenShift` reaches the wire through `from_cash_error`
//! (`src/presentation/http/v3/commands/cash.rs`) as `AppError::Domain(<its sentence>)`, which answers
//! `400 validationFailed` with that sentence. The ticket's "400 `noOpenShift`" is asserted as status 400 plus
//! `ShiftError::NoOpenShift`'s sentence; the error code word is not pinned (changing `from_cash_error` is outside
//! the ticket's file scope). `AppError::Forbidden` answers `403 outOfScope`. The transfer status the ticket calls
//! IN_TRANSIT is stored as `DISPATCHED` (`TransferStatus::Dispatched`).
//!
//! | Row | Test |
//! |---|---|
//! | T1 | `t1_receive_into_closed_drawer_is_refused_and_writes_nothing` |
//! | T2 | `t2_receive_into_open_drawer_by_its_teller_is_received` (kept) |
//! | T3 | `t3_receive_by_another_teller_keeps_the_existing_refusal` (kept) |
//! | T4 | `t4_receive_naming_another_branch_is_out_of_scope` (kept) |
//! | T5 | `t5_dispatch_naming_another_branch_than_the_source_is_out_of_scope` |
//! | T6 | `t6_dispatch_naming_the_source_branch_is_dispatched` (kept) |
//! | T7 | `t7_drawer_to_safe_and_branch_kinds_still_dispatch_and_receive` (kept) |
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
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_testdb::people::{RoleRow, seed_person};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B02: &str = "XC-B02";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// B01's drawer: the OPEN shift of `Cast::teller`.
const SHIFT: &str = "SH-XC-B01";

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
        let schema = unique("xfer2704");
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
        for branch in [B01, B02, B09] {
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

    /// A user holding `role` on each of `branches` (`&[]` = organisation-wide).
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

    /// Opens B01's drawer `SHIFT` for `teller`.
    async fn open_shift(&self, teller: &Who) {
        self.exec(&format!(
            "INSERT INTO shifts (id, organization_id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('{SHIFT}', '{ORG}'::uuid, '{B01}', 'OPEN', now(), 'test', 'open-{SHIFT}', '{}'::uuid)",
            teller.id
        ))
        .await;
    }

    async fn close_shift(&self) {
        self.exec(&format!(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now(), closed_by = 'test' WHERE id = '{SHIFT}'"
        ))
        .await;
    }

    /// The stored state of a transfer: status, received_at set?, ledger postings written for it.
    async fn stored(&self, id: &str) -> Stored {
        let (status, received): (String, bool) = sqlx::query_as(
            "SELECT status, received_at IS NOT NULL FROM cash_transfers WHERE id = $1::uuid",
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
        Stored {
            status,
            received,
            postings,
        }
    }

    /// `branch_versions.version` of `branch` (0 when the branch has never run a command).
    async fn version(&self, branch: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE((SELECT version::bigint FROM branch_versions WHERE branch_id = $1), 0)",
        )
        .bind(branch)
        .fetch_one(&self.pool)
        .await
        .expect("the branch version")
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
    received: bool,
    postings: i64,
}

/// Puts USD `amount` into `place` through the ledger, which dispatch checks stock against.
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
                    source_id: format!("xfer2704-fund:{}", FUND.fetch_add(1, Ordering::Relaxed)),
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

/// A signed-in person: their user id and the spec that signs them in (`id|ROLE|B1,B2` or `id|ROLE|*`).
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
    // A user scoped to several branches signs in to one of them (SB-1 session branch): the first listed. Without
    // it, `authenticate_caller` refuses a multi-branch caller `outOfScope` before the command runs.
    let session = (branches != "*")
        .then(|| branches.split(',').next().map(BranchId::new))
        .flatten();
    Some(User::new(id, ORG, scopes, None).with_branch(session))
}

fn legacy_of(spec: &str) -> Sessions {
    let mut parts = spec.splitn(3, '|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(id), Some(_), Some("*")) => Sessions {
            owner: Some(Actor::owner(id)),
            staff: None,
        },
        (Some(id), Some(_), Some(branches)) => Sessions {
            owner: None,
            staff: Actor::staff(
                id,
                OldRole::Finance,
                BranchId::new(branches.split(',').next().unwrap_or(branches)),
            ),
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

/// One POST through the whole route table, as `who`.
async fn post(room: &Room, who: &Who, uri: &str, body: Value) -> Answer {
    let pool = room.pool.clone();
    let spec = who.spec.clone();
    let path = uri.to_owned();
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
        Ok(a) => a,
        Err(e) => panic!("POST {uri}: the request task failed: {e}"),
    }
}

fn code(a: &Answer) -> &str {
    a.body["error"]["code"].as_str().unwrap_or("")
}

fn message(a: &Answer) -> &str {
    a.body["error"]["message"].as_str().unwrap_or("")
}

fn assert_status(a: &Answer, want: &str, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    assert_eq!(a.body["status"], want, "{what}: {}", a.text);
}

fn assert_out_of_scope(a: &Answer, what: &str) {
    assert!(
        a.status == StatusCode::FORBIDDEN && code(a) == "outOfScope",
        "{what}: want 403 outOfScope, got {} {}",
        a.status,
        a.text
    );
}

// ------------------------------------------------------------ the people and transfers

struct Cast {
    /// BRANCH_MANAGER of B01: creates, submits and dispatches B01's moves.
    bm01: Who,
    /// A second BRANCH_MANAGER of B01: approves B01's transfers.
    approver01: Who,
    bm02: Who,
    approver02: Who,
    /// BRANCH_MANAGER of B09: receives BRANCH transfers into B09.
    bm09: Who,
    /// TELLER of B01 and the teller of `SHIFT`.
    teller: Who,
    /// Another TELLER of B01, not the teller of `SHIFT`.
    other_teller: Who,
    /// FINANCE scoped to B01 and B02 (allowed: `crates/xc-auth/src/domain/access/scope_rule.rs`).
    finance: Who,
    /// FINANCE scoped to B01 and B09.
    finance09: Who,
}

async fn cast(room: &Room) -> Cast {
    let executive = room.person(Role::Executive, &[]).await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '300.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(&executive.id)
    .execute(&room.pool)
    .await
    .expect("a transfer policy version");
    let c = Cast {
        bm01: room.person(Role::BranchManager, &[B01]).await,
        approver01: room.person(Role::BranchManager, &[B01]).await,
        bm02: room.person(Role::BranchManager, &[B02]).await,
        approver02: room.person(Role::BranchManager, &[B02]).await,
        bm09: room.person(Role::BranchManager, &[B09]).await,
        teller: room.person(Role::Teller, &[B01]).await,
        other_teller: room.person(Role::Teller, &[B01]).await,
        finance: room.person(Role::Finance, &[B01, B02]).await,
        finance09: room.person(Role::Finance, &[B01, B09]).await,
    };
    room.open_shift(&c.teller).await;
    c
}

/// Creates a transfer over HTTP as `creator` naming `branch`, submits it, and approves it as `approver`.
async fn approved(room: &Room, creator: &Who, approver: &Who, branch: &str, ends: Value) -> String {
    let usd50 = room.usd50().await;
    let mut body = json!({
        "branch": branch,
        "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "2" }],
        "note": "test move"
    });
    for (k, v) in ends.as_object().expect("an object") {
        body[k] = v.clone();
    }
    let a = post(room, creator, "/api/cash-transfers", body).await;
    assert_status(&a, "DRAFT", "transferCreate");
    let id = a.body["id"].as_str().expect("an id").to_owned();
    let s = post(
        room,
        creator,
        &format!("/api/cash-transfers/{id}/submit"),
        json!({ "branch": branch }),
    )
    .await;
    assert_eq!(s.status, StatusCode::OK, "transferSubmit: {}", s.text);
    TransferFlow::pg(room.pool.clone())
        .decide(
            &user_of(&approver.spec).expect("the approver"),
            &id,
            Outcome::Approved,
            "ok",
            Utc::now(),
        )
        .await
        .expect("a branch manager of the source approves within the limit");
    id
}

async fn dispatch(room: &Room, who: &Who, id: &str, branch: &str) -> Answer {
    post(
        room,
        who,
        &format!("/api/cash-transfers/{id}/dispatch"),
        json!({ "branch": branch }),
    )
    .await
}

async fn receive(room: &Room, who: &Who, id: &str, branch: &str) -> Answer {
    let usd50 = room.usd50().await;
    post(
        room,
        who,
        &format!("/api/cash-transfers/{id}/receive"),
        json!({
            "branch": branch,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qtyReceived": "2" }]
        }),
    )
    .await
}

/// A SAFE_TO_DRAWER move at B01 into `SHIFT`, approved and dispatched by B01's manager (DISPATCHED).
async fn safe_to_drawer_dispatched(room: &Room, c: &Cast) -> String {
    let id = approved(
        room,
        &c.bm01,
        &c.approver01,
        B01,
        json!({ "kind": "SAFE_TO_DRAWER", "toShiftId": SHIFT }),
    )
    .await;
    assert_status(
        &dispatch(room, &c.bm01, &id, B01).await,
        "DISPATCHED",
        "dispatch SAFE_TO_DRAWER",
    );
    id
}

/// A BRANCH transfer B02 → B09, approved (APPROVED).
async fn b02_branch_transfer_approved(room: &Room, c: &Cast) -> String {
    approved(
        room,
        &c.bm02,
        &c.approver02,
        B02,
        json!({ "kind": "BRANCH", "toBranchId": B09 }),
    )
    .await
}

// ------------------------------------------------------------ T1–T4: receive into a drawer

/// T1: the destination shift was CLOSED after dispatch. Receive is refused with `ShiftError::NoOpenShift` (400),
/// the transfer stays DISPATCHED, and no receive posting is written. Main answers 200 RECEIVED.
#[actix_web::test]
async fn t1_receive_into_closed_drawer_is_refused_and_writes_nothing() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = safe_to_drawer_dispatched(&room, &c).await;
    room.close_shift().await;
    let before = room.stored(&id).await;

    let a = receive(&room, &c.teller, &id, B01).await;

    assert!(
        a.status == StatusCode::BAD_REQUEST && message(&a) == ShiftError::NoOpenShift.to_string(),
        "T1: receiving into a CLOSED drawer must answer 400 with ShiftError::NoOpenShift's sentence; got {} {}",
        a.status,
        a.text
    );
    let after = room.stored(&id).await;
    assert_eq!(
        after.status, "DISPATCHED",
        "T1: the transfer stays in transit"
    );
    assert!(!after.received, "T1: received_at stays unset");
    assert_eq!(
        after.postings, before.postings,
        "T1: no ledger posting is written into the closed drawer"
    );
    room.finish().await;
}

/// T2 (kept): the drawer is OPEN and the receiver is its teller: 200 RECEIVED, the cash is posted.
#[actix_web::test]
async fn t2_receive_into_open_drawer_by_its_teller_is_received() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = safe_to_drawer_dispatched(&room, &c).await;
    let before = room.stored(&id).await;

    let a = receive(&room, &c.teller, &id, B01).await;

    assert_status(&a, "RECEIVED", "T2");
    let after = room.stored(&id).await;
    assert!(after.received, "T2: received_at is set");
    assert!(
        after.postings > before.postings,
        "T2: the receive posts into the drawer"
    );
    room.finish().await;
}

/// T3 (kept): another teller of the same branch is refused with the existing person rule, and nothing moves.
#[actix_web::test]
async fn t3_receive_by_another_teller_keeps_the_existing_refusal() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = safe_to_drawer_dispatched(&room, &c).await;
    let before = room.stored(&id).await;

    let a = receive(&room, &c.other_teller, &id, B01).await;

    assert!(
        a.status == StatusCode::BAD_REQUEST
            && message(&a) == "only the shift's teller may receive into drawer",
        "T3: want the existing 400 person-rule refusal; got {} {}",
        a.status,
        a.text
    );
    assert_eq!(room.stored(&id).await, before, "T3: nothing moves");
    room.finish().await;
}

/// T4 (kept): a receive naming a branch that is not the destination is 403 `outOfScope`, and nothing moves.
#[actix_web::test]
async fn t4_receive_naming_another_branch_is_out_of_scope() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = safe_to_drawer_dispatched(&room, &c).await;
    let before = room.stored(&id).await;

    let a = receive(&room, &c.finance09, &id, B09).await;

    assert_out_of_scope(&a, "T4");
    assert_eq!(room.stored(&id).await, before, "T4: nothing moves");
    room.finish().await;
}

// ------------------------------------------------------------ T5–T6: dispatch from the source

/// T5: FINANCE@B01+B02 names B01 and dispatches a transfer whose source is B02. Refused 403 `outOfScope`; the
/// transfer stays APPROVED with no posting, and neither B01's nor B02's `branch_versions` moves. Main answers 200.
#[actix_web::test]
async fn t5_dispatch_naming_another_branch_than_the_source_is_out_of_scope() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = b02_branch_transfer_approved(&room, &c).await;
    let before = room.stored(&id).await;
    let (v01, v02) = (room.version(B01).await, room.version(B02).await);

    let a = dispatch(&room, &c.finance, &id, B01).await;

    assert_out_of_scope(&a, "T5");
    let after = room.stored(&id).await;
    assert_eq!(after.status, "APPROVED", "T5: the status is unchanged");
    assert_eq!(after.postings, before.postings, "T5: no posting is written");
    assert_eq!(
        room.version(B01).await,
        v01,
        "T5: B01's version does not move"
    );
    assert_eq!(
        room.version(B02).await,
        v02,
        "T5: B02's version does not move"
    );
    room.finish().await;
}

/// T6 (kept): the same user names the source branch B02: 200 DISPATCHED.
#[actix_web::test]
async fn t6_dispatch_naming_the_source_branch_is_dispatched() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let id = b02_branch_transfer_approved(&room, &c).await;

    let a = dispatch(&room, &c.finance, &id, B02).await;

    assert_status(&a, "DISPATCHED", "T6");
    room.finish().await;
}

// ------------------------------------------------------------ T7: other kinds

/// T7 (kept): DRAWER_TO_SAFE at B01 and BRANCH B01 → B09 still dispatch at the source and are received.
#[actix_web::test]
async fn t7_drawer_to_safe_and_branch_kinds_still_dispatch_and_receive() {
    let room = Room::start().await;
    let c = cast(&room).await;
    fund(&room.pool, CashPlace::Drawer(SHIFT.to_owned()), "500.00").await;

    let d2s = approved(
        &room,
        &c.bm01,
        &c.approver01,
        B01,
        json!({ "kind": "DRAWER_TO_SAFE", "fromShiftId": SHIFT }),
    )
    .await;
    assert_status(
        &dispatch(&room, &c.bm01, &d2s, B01).await,
        "DISPATCHED",
        "T7 DRAWER_TO_SAFE dispatch",
    );
    assert_status(
        &receive(&room, &c.bm01, &d2s, B01).await,
        "RECEIVED",
        "T7 DRAWER_TO_SAFE receive",
    );

    let branch = approved(
        &room,
        &c.bm01,
        &c.approver01,
        B01,
        json!({ "kind": "BRANCH", "toBranchId": B09 }),
    )
    .await;
    assert_status(
        &dispatch(&room, &c.bm01, &branch, B01).await,
        "DISPATCHED",
        "T7 BRANCH dispatch",
    );
    assert_status(
        &receive(&room, &c.bm09, &branch, B09).await,
        "RECEIVED",
        "T7 BRANCH receive",
    );
    room.finish().await;
}
