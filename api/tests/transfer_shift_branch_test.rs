//! Up-front tests: a cash transfer's drawer end must be an OPEN shift at the transfer's own branch when it is created.
//!
//! Written before the fix by the test-writer for the xchange BE lane controller claude/7ca80c. Do not edit, delete or
//! weaken these tests; add your own in another file.
//!
//! **The bug** (main `3bac87e0a`): `SafeMove::check_create` (`crates/xc-cash/src/domain/transfer/kinds/safe.rs`)
//! only checks that `from_shift` / `to_shift` are present, and `TransferFlow::create_in`
//! (`crates/xc-cash/src/application/transfer/transfer_flow.rs`) hands them to `insert`
//! (`crates/xc-cash/src/infrastructure/transfer_repository.rs`) unread. The only guard is the FK
//! `cash_transfers.from_shift_id REFERENCES shifts(id)` (`migrations/20261012100000_cash.sql`), which compares no
//! branch and no status. A B01 manager can therefore create a DRAWER_TO_SAFE out of B09's drawer (dispatch then posts
//! `Drawer(<B09 shift>) → InTransit` and receipt lands it in B01's safe), and an unknown shift id is a 500 from the FK.
//! The sibling paths already check: a count's shift (`count_service.rs`, `status = 'OPEN' AND branch_id = $2`) and a
//! hand-over's (`handover_service.rs`) both refuse with `ShiftError::NoOpenShift`.
//!
//! **Ruling.** DRAWER_TO_SAFE's `from_shift` and SAFE_TO_DRAWER's `to_shift` must name an OPEN shift at the transfer's
//! branch (`from_branch`; safe moves have no `to_branch`). Anything else is refused the way the count path refuses a
//! shift that is not open at the branch: `ShiftError::NoOpenShift`, which reaches the wire through `from_cash_error`
//! (`src/presentation/http/v3/commands/cash.rs`) as `AppError::Domain(<its sentence>)` = `400 validationFailed` with
//! that sentence (the receive path, `transfer_flow.rs` `receive_on`, already answers a drawer that is not OPEN the same
//! way). These tests pin status 400 plus that sentence; they do not pin the error code word. Nothing is stored.
//!
//! | Row | Test | Main today |
//! |---|---|---|
//! | T1 | `t1_drawer_to_safe_out_of_another_branchs_open_shift_is_refused_and_stores_nothing` | 200 DRAFT |
//! | T2 | `t2_drawer_to_safe_out_of_an_unknown_shift_is_refused_and_stores_nothing` | 500 (FK) |
//! | T3 | `t3_safe_to_drawer_into_another_branchs_open_shift_is_refused_and_stores_nothing` | 200 DRAFT |
//! | T4 | `t4_drawer_to_safe_out_of_this_branchs_open_shift_is_created` (kept) | 200 DRAFT |
//! | T5 | `t5_drawer_to_safe_out_of_this_branchs_closed_shift_is_refused_and_stores_nothing` | 200 DRAFT |
//! | T6 | `t6_safe_to_drawer_into_this_branchs_closed_shift_is_refused_and_stores_nothing` | 200 DRAFT |
//! | T7 | `t7_safe_to_drawer_into_this_branchs_open_shift_is_created` (kept) | 200 DRAFT |
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips. Each test works in a schema
//! of its own, with every migration applied.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
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
/// B01's drawer: the OPEN shift of `Cast::teller01`.
const SHIFT01: &str = "SH-XC-B01";
/// B09's drawer: the OPEN shift of `Cast::teller09`.
const SHIFT09: &str = "SH-XC-B09";
/// No such shift.
const NO_SHIFT: &str = "SH-nope";

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
        let schema = unique("xfershbr");
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

    /// Opens drawer `shift` at `branch` for `teller_id`.
    async fn open_shift(&self, shift: &str, branch: &str, teller_id: &str) {
        self.exec(&format!(
            "INSERT INTO shifts (id, organization_id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('{shift}', '{ORG}'::uuid, '{branch}', 'OPEN', now(), 'test', 'open-{shift}', '{teller_id}'::uuid)"
        ))
        .await;
    }

    async fn close_shift(&self, shift: &str) {
        self.exec(&format!(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now(), closed_by = 'test' WHERE id = '{shift}'"
        ))
        .await;
    }

    /// (transfers, transfer lines) stored in the schema.
    async fn stored(&self) -> (i64, i64) {
        (
            self.count("SELECT count(*) FROM cash_transfers").await,
            self.count("SELECT count(*) FROM cash_transfer_lines").await,
        )
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

/// A signed-in person: the spec that signs them in (`id|ROLE|B1,B2` or `id|ROLE|*`).
struct Who {
    spec: String,
}

impl Who {
    fn id(&self) -> &str {
        self.spec.split('|').next().expect("an id")
    }
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

fn message(a: &Answer) -> &str {
    a.body["error"]["message"].as_str().unwrap_or("")
}

/// The refusal the count path gives a shift that is not OPEN at the branch: 400 with `ShiftError::NoOpenShift`'s
/// sentence.
fn assert_no_open_shift(a: &Answer, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST && message(a) == ShiftError::NoOpenShift.to_string(),
        "{what}: want 400 with ShiftError::NoOpenShift's sentence ({:?}); got {} {}",
        ShiftError::NoOpenShift.to_string(),
        a.status,
        a.text
    );
}

fn assert_created(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    assert_eq!(a.body["status"], "DRAFT", "{what}: {}", a.text);
}

// ------------------------------------------------------------ the people and transfers

struct Cast {
    /// BRANCH_MANAGER of B01: creates B01's moves.
    bm01: Who,
}

async fn cast(room: &Room) -> Cast {
    let executive = room.person(Role::Executive, &[]).await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '300.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(executive.id())
    .execute(&room.pool)
    .await
    .expect("a transfer policy version");
    let bm01 = room.person(Role::BranchManager, &[B01]).await;
    let teller01 = room.person(Role::Teller, &[B01]).await;
    let teller09 = room.person(Role::Teller, &[B09]).await;
    room.open_shift(SHIFT01, B01, teller01.id()).await;
    room.open_shift(SHIFT09, B09, teller09.id()).await;
    Cast { bm01 }
}

/// POST /api/cash-transfers as B01's manager, naming B01, with the safe move's `ends`.
async fn create_at_b01(room: &Room, c: &Cast, ends: Value) -> Answer {
    let usd50 = room.usd50().await;
    let mut body = json!({
        "branch": B01,
        "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "2" }],
        "note": "test move"
    });
    for (k, v) in ends.as_object().expect("an object") {
        body[k] = v.clone();
    }
    post(room, &c.bm01, "/api/cash-transfers", body).await
}

// ------------------------------------------------------------ T1–T3: a shift that is not this branch's

/// T1: B01's manager creates DRAWER_TO_SAFE out of B09's OPEN drawer. Refused like the count path (400
/// `ShiftError::NoOpenShift`), and no transfer or line is stored. Main answers 200 DRAFT.
#[actix_web::test]
async fn t1_drawer_to_safe_out_of_another_branchs_open_shift_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let before = room.stored().await;

    let a = create_at_b01(
        &room,
        &c,
        json!({ "kind": "DRAWER_TO_SAFE", "fromShiftId": SHIFT09 }),
    )
    .await;

    assert_no_open_shift(&a, "T1: DRAWER_TO_SAFE at B01 out of B09's drawer");
    assert_eq!(
        room.stored().await,
        before,
        "T1: no transfer and no line is stored"
    );
    room.finish().await;
}

/// T2: DRAWER_TO_SAFE out of a shift that does not exist. The same refusal, not a 500 from the FK; nothing stored.
#[actix_web::test]
async fn t2_drawer_to_safe_out_of_an_unknown_shift_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let before = room.stored().await;

    let a = create_at_b01(
        &room,
        &c,
        json!({ "kind": "DRAWER_TO_SAFE", "fromShiftId": NO_SHIFT }),
    )
    .await;

    assert_no_open_shift(&a, "T2: DRAWER_TO_SAFE out of an unknown shift");
    assert_eq!(
        room.stored().await,
        before,
        "T2: no transfer and no line is stored"
    );
    room.finish().await;
}

/// T3: B01's manager creates SAFE_TO_DRAWER into B09's OPEN drawer. Refused 400 `ShiftError::NoOpenShift`; nothing
/// stored. Main answers 200 DRAFT.
#[actix_web::test]
async fn t3_safe_to_drawer_into_another_branchs_open_shift_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let c = cast(&room).await;
    let before = room.stored().await;

    let a = create_at_b01(
        &room,
        &c,
        json!({ "kind": "SAFE_TO_DRAWER", "toShiftId": SHIFT09 }),
    )
    .await;

    assert_no_open_shift(&a, "T3: SAFE_TO_DRAWER at B01 into B09's drawer");
    assert_eq!(
        room.stored().await,
        before,
        "T3: no transfer and no line is stored"
    );
    room.finish().await;
}

// ------------------------------------------------------------ T4: this branch's OPEN shift (kept)

/// T4 (kept): DRAWER_TO_SAFE out of B01's own OPEN drawer is created as today: 200 DRAFT, one row naming the shift.
#[actix_web::test]
async fn t4_drawer_to_safe_out_of_this_branchs_open_shift_is_created() {
    let room = Room::start().await;
    let c = cast(&room).await;

    let a = create_at_b01(
        &room,
        &c,
        json!({ "kind": "DRAWER_TO_SAFE", "fromShiftId": SHIFT01 }),
    )
    .await;

    assert_created(&a, "T4");
    assert_eq!(
        room.count(&format!(
            "SELECT count(*) FROM cash_transfers WHERE from_branch_id = '{B01}' AND from_shift_id = '{SHIFT01}' \
             AND kind = 'DRAWER_TO_SAFE' AND status = 'DRAFT'"
        ))
        .await,
        1,
        "T4: the transfer is stored out of B01's drawer"
    );
    room.finish().await;
}

// ------------------------------------------------------------ T5–T6: this branch's CLOSED shift

/// T5: DRAWER_TO_SAFE out of B01's own drawer after it CLOSED. Refused 400 `ShiftError::NoOpenShift`; nothing stored.
#[actix_web::test]
async fn t5_drawer_to_safe_out_of_this_branchs_closed_shift_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let c = cast(&room).await;
    room.close_shift(SHIFT01).await;
    let before = room.stored().await;

    let a = create_at_b01(
        &room,
        &c,
        json!({ "kind": "DRAWER_TO_SAFE", "fromShiftId": SHIFT01 }),
    )
    .await;

    assert_no_open_shift(&a, "T5: DRAWER_TO_SAFE out of a CLOSED drawer");
    assert_eq!(
        room.stored().await,
        before,
        "T5: no transfer and no line is stored"
    );
    room.finish().await;
}

/// T6: SAFE_TO_DRAWER into B01's own drawer after it CLOSED. Refused 400 `ShiftError::NoOpenShift`; nothing stored.
#[actix_web::test]
async fn t6_safe_to_drawer_into_this_branchs_closed_shift_is_refused_and_stores_nothing() {
    let room = Room::start().await;
    let c = cast(&room).await;
    room.close_shift(SHIFT01).await;
    let before = room.stored().await;

    let a = create_at_b01(
        &room,
        &c,
        json!({ "kind": "SAFE_TO_DRAWER", "toShiftId": SHIFT01 }),
    )
    .await;

    assert_no_open_shift(&a, "T6: SAFE_TO_DRAWER into a CLOSED drawer");
    assert_eq!(
        room.stored().await,
        before,
        "T6: no transfer and no line is stored"
    );
    room.finish().await;
}

// ------------------------------------------------------------ T7: SAFE_TO_DRAWER into this branch's OPEN shift (kept)

/// T7 (kept): SAFE_TO_DRAWER into B01's own OPEN drawer is created as today: 200 DRAFT, one row naming the shift.
#[actix_web::test]
async fn t7_safe_to_drawer_into_this_branchs_open_shift_is_created() {
    let room = Room::start().await;
    let c = cast(&room).await;

    let a = create_at_b01(
        &room,
        &c,
        json!({ "kind": "SAFE_TO_DRAWER", "toShiftId": SHIFT01 }),
    )
    .await;

    assert_created(&a, "T7");
    assert_eq!(
        room.count(&format!(
            "SELECT count(*) FROM cash_transfers WHERE from_branch_id = '{B01}' AND to_shift_id = '{SHIFT01}' \
             AND kind = 'SAFE_TO_DRAWER' AND status = 'DRAFT'"
        ))
        .await,
        1,
        "T7: the transfer is stored into B01's drawer"
    );
    room.finish().await;
}
