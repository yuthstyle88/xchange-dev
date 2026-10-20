//! Up-front tests for #2703: `handoverStart` (`POST /api/handovers`) and a CLOSE `countSubmit`
//! (`POST /api/cash-counts`) must check the receiver and the shift before writing anything.
//!
//! Written before the fix by the test-writer claude/dell-claude-1, on the BE lane controller claude/1656900d's
//! order. Do not edit, delete or weaken these tests; add your own in another file.
//!
//! **The bugs** (main `488513e0`):
//! - `HandoverService::start_in` (`crates/xc-cash/src/application/handover/handover_service.rs`) refuses only
//!   `receiver_id == actor.user_id`, then binds `receiver_id` as `$5::uuid` into `INSERT INTO handovers`: a malformed
//!   or unknown id answers 500, and a user of another organisation or without `ShiftOperate` at the branch gets a
//!   PENDING handover nobody can accept.
//! - `start_in` and `CashCountService::submit_in` (`crates/xc-cash/src/application/count/count_service.rs`) look the
//!   shift up with `SELECT teller_id::text FROM shifts WHERE id = $1`: no `status = 'OPEN'`, no `branch_id`.
//!
//! **Wire shapes asserted.**
//! - H1–H4 (controller ruling on #2703, issuecomment-6224799454): `400 validationFailed` with `error.field ==
//!   "receiverId"`, and nothing written.
//! - H7–H9: `ShiftError::NoOpenShift` reaches the wire through `from_cash_error`
//!   (`src/presentation/http/v3/commands/cash.rs`) as `AppError::Domain(<its sentence>)` = `400 validationFailed`
//!   with that sentence. The ticket's "400 `noOpenShift`" is asserted as status 400 plus the sentence; the code word
//!   is not pinned.
//! - H6 keeps today's `CashError::SameCounter` refusal (400 with its sentence).
//!
//! | Row | Test |
//! |---|---|
//! | H1 | `h1_receiver_not_a_uuid_is_refused_on_receiver_id` |
//! | H2 | `h2_receiver_of_no_user_is_refused_on_receiver_id` |
//! | H3 | `h3_receiver_of_another_organisation_is_refused_on_receiver_id` |
//! | H4 | `h4_receiver_without_shift_operate_at_the_branch_is_refused_on_receiver_id` |
//! | H5 | `h5_receiver_teller_of_the_branch_is_pending` (kept) |
//! | H6 | `h6_receiver_is_the_teller_keeps_same_counter` (kept) |
//! | H7 | `h7_handover_from_own_closed_shift_is_no_open_shift` |
//! | H8 | `h8_handover_from_own_open_shift_at_another_branch_is_no_open_shift` |
//! | H9 | `h9_close_count_on_own_closed_shift_is_no_open_shift` |
//! | H10 | `h10_open_and_safe_counts_are_unchanged` (kept) |
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

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B02: &str = "XC-B02";
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
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
        let schema = unique("ho2703");
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
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase) \
             VALUES ($1, 'OPEN')",
        )
        .bind(B01)
        .execute(&pool)
        .await
        .expect("the branch's safe");
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

        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(B02)
            .execute(&pool)
            .await
            .expect("insert branch B02");
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase) \
             VALUES ($1, 'OPEN')",
        )
        .bind(B02)
        .execute(&pool)
        .await
        .expect("B02's safe");
        let safe_b02 = || xc_cash::domain::posting::CashPlace::Safe(BranchId::new(B02));
        lg4_fund(
            &pool,
            xc_cash::domain::posting::CashPlace::Bank,
            safe_b02(),
            "USD",
            "500.00",
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

// ------------------------------------------------------------ helpers for #2703

use xc_cash::domain::error::CashError;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;

impl Room {
    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// Everything a refused request must leave untouched.
    async fn written(&self) -> (i64, i64) {
        (
            self.scalar("SELECT count(*) FROM handovers").await,
            self.scalar("SELECT count(*) FROM cash_counts").await,
        )
    }

    /// A shift row of `teller` at `branch` with `status`, seeded directly (`id` is returned).
    async fn seed_shift(&self, teller: &Who, branch: &str, status: &str) -> String {
        let id = unique("SH");
        let (closed_at, closed_by) = if status == "CLOSED" {
            ("now()", "'test'")
        } else {
            ("NULL", "NULL")
        };
        self.exec(&format!(
            "INSERT INTO shifts (id, organization_id, branch_id, status, opened_at, opened_by, open_request_id, \
             teller_id, closed_at, closed_by) \
             VALUES ('{id}', '{ORG}'::uuid, '{branch}', '{status}', now() - interval '1 day', 'test', 'open-{id}', \
             '{}'::uuid, {closed_at}, {closed_by})",
            teller.id
        ))
        .await;
        id
    }

    /// An ACTIVE user of organisation B (with its own branch and a TELLER role there).
    async fn person_of_org_b(&self) -> String {
        self.exec(&format!(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ('{ORG_B}'::uuid, 'ORGB', 'Asia/Bangkok', now()) ON CONFLICT DO NOTHING"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO branches (id, name, organization_id) VALUES ('ORGB-B01', 'B01', '{ORG_B}'::uuid) \
             ON CONFLICT DO NOTHING"
        ))
        .await;
        xc_testdb::people::seed_person(
            &mut self.pool.acquire().await.expect("acquire"),
            ORG_B,
            &unique("orgb-teller"),
            &[xc_testdb::people::RoleRow {
                role_code: "TELLER",
                branch_id: Some("ORGB-B01"),
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("insert an org-B user")
    }
}

fn field(a: &Answer) -> &str {
    a.body["error"]["field"].as_str().unwrap_or("")
}

fn message(a: &Answer) -> &str {
    a.body["error"]["message"].as_str().unwrap_or("")
}

async fn handover(room: &Room, teller: &Who, shift: &str, receiver: &str) -> Answer {
    post(
        room,
        teller,
        "/api/handovers",
        json!({
            "branch": B01,
            "fromShiftId": shift,
            "receiverId": receiver,
            "lines": [{ "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "1" }]
        }),
    )
    .await
}

/// H1–H4: `400 validationFailed` on field `receiverId`, and no handover or count row written.
async fn assert_refused_on_receiver(room: &Room, receiver: &str, what: &str) {
    let (teller, _, shift) = opened_shift(room).await;
    let before = room.written().await;
    let a = handover(room, &teller, &shift, receiver).await;
    assert!(
        a.status == StatusCode::BAD_REQUEST
            && code(&a) == "validationFailed"
            && field(&a) == "receiverId",
        "{what}: want 400 validationFailed on receiverId, got {} {}",
        a.status,
        a.text
    );
    assert_eq!(room.written().await, before, "{what}: nothing is written");
}

fn assert_no_open_shift(a: &Answer, what: &str) {
    assert!(
        a.status == StatusCode::BAD_REQUEST && message(a) == ShiftError::NoOpenShift.to_string(),
        "{what}: want 400 with ShiftError::NoOpenShift's sentence, got {} {}",
        a.status,
        a.text
    );
}

// ------------------------------------------------------------ H1–H6: the receiver

#[actix_web::test]
async fn h1_receiver_not_a_uuid_is_refused_on_receiver_id() {
    let room = Room::start().await;
    assert_refused_on_receiver(&room, "not-a-uuid", "H1").await;
    room.finish().await;
}

#[actix_web::test]
async fn h2_receiver_of_no_user_is_refused_on_receiver_id() {
    let room = Room::start().await;
    assert_refused_on_receiver(&room, "00000000-0000-0000-0000-00000000dead", "H2").await;
    room.finish().await;
}

#[actix_web::test]
async fn h3_receiver_of_another_organisation_is_refused_on_receiver_id() {
    let room = Room::start().await;
    let other = room.person_of_org_b().await;
    assert_refused_on_receiver(&room, &other, "H3").await;
    room.finish().await;
}

/// H4: a user of this organisation whose `ShiftOperate` is at B02, not B01.
#[actix_web::test]
async fn h4_receiver_without_shift_operate_at_the_branch_is_refused_on_receiver_id() {
    let room = Room::start().await;
    let elsewhere = room.person(Role::Teller, Some(B02)).await;
    assert_refused_on_receiver(&room, &elsewhere.id, "H4 (teller of B02)").await;
    room.finish().await;
}

/// H4 also covers a user of B01 with no `ShiftOperate` at all (FINANCE).
#[actix_web::test]
async fn h4_receiver_finance_of_the_branch_is_refused_on_receiver_id() {
    let room = Room::start().await;
    let finance = room.person(Role::Finance, Some(B01)).await;
    assert_refused_on_receiver(&room, &finance.id, "H4 (FINANCE of B01)").await;
    room.finish().await;
}

#[actix_web::test]
async fn h5_receiver_teller_of_the_branch_is_pending() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let a = handover(&room, &teller, &shift, &receiver.id).await;
    assert_eq!(ok(&a, "H5")["status"], "PENDING", "{}", a.text);
    room.finish().await;
}

#[actix_web::test]
async fn h6_receiver_is_the_teller_keeps_same_counter() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let before = room.written().await;
    let a = handover(&room, &teller, &shift, &teller.id).await;
    assert!(
        a.status == StatusCode::BAD_REQUEST && message(&a) == CashError::SameCounter.to_string(),
        "H6: want today's SameCounter refusal, got {} {}",
        a.status,
        a.text
    );
    assert_eq!(room.written().await, before, "H6: nothing is written");
    room.finish().await;
}

// ------------------------------------------------------------ H7–H10: the shift

/// H7: T has an OPEN shift at B01 and names an older CLOSED shift of their own.
#[actix_web::test]
async fn h7_handover_from_own_closed_shift_is_no_open_shift() {
    let room = Room::start().await;
    let (teller, _, _open) = opened_shift(&room).await;
    let closed = room.seed_shift(&teller, B01, "CLOSED").await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let before = room.written().await;

    let a = handover(&room, &teller, &closed, &receiver.id).await;

    assert_no_open_shift(&a, "H7");
    assert_eq!(room.written().await, before, "H7: nothing is written");
    room.finish().await;
}

/// H8: T names their own OPEN shift at B02 while the request names B01.
#[actix_web::test]
async fn h8_handover_from_own_open_shift_at_another_branch_is_no_open_shift() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let at_b02 = room.seed_shift(&teller, B02, "OPEN").await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let before = room.written().await;

    let a = handover(&room, &teller, &at_b02, &receiver.id).await;

    assert_no_open_shift(&a, "H8");
    assert_eq!(room.written().await, before, "H8: nothing is written");
    room.finish().await;
}

/// H9: a CLOSE count naming T's own CLOSED shift.
#[actix_web::test]
async fn h9_close_count_on_own_closed_shift_is_no_open_shift() {
    let room = Room::start().await;
    let (teller, _, _open) = opened_shift(&room).await;
    let closed = room.seed_shift(&teller, B01, "CLOSED").await;
    let before = room.written().await;

    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        json!({
            "branch": B01,
            "kind": "CLOSE",
            "shiftId": closed,
            "lines": [{ "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "2" }]
        }),
    )
    .await;

    assert_no_open_shift(&a, "H9");
    assert_eq!(room.written().await, before, "H9: nothing is written");
    room.finish().await;
}

/// H10 (kept): an OPEN count (no shift) by a teller and a SAFE count by a CashManage holder still answer 200.
#[actix_web::test]
async fn h10_open_and_safe_counts_are_unchanged() {
    use xc_auth::domain::access::{Permission, RoleGrants};

    let room = Room::start().await;
    assert!(
        RoleGrants::seed(Role::Teller).contains(&Permission::ShiftOperate)
            && RoleGrants::seed(Role::Finance).contains(&Permission::CashManage),
        "§11.2: TELLER holds ShiftOperate and FINANCE holds CashManage"
    );
    let teller = room.person(Role::Teller, Some(B01)).await;
    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        open_count_body(&room).await,
    )
    .await;
    ok(&a, "H10 OPEN count");

    let finance = room.person(Role::Finance, Some(B01)).await;
    let a = post(
        &room,
        &finance,
        "/api/cash-counts",
        json!({
            "branch": B01,
            "kind": "SAFE",
            "lines": [{ "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "1" }]
        }),
    )
    .await;
    ok(&a, "H10 SAFE count");
    room.finish().await;
}
