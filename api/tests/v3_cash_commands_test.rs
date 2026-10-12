//! Ticket #624 (CA-8): the cash commands and views of `docs/design/blueprint/cash-shift-transfer.md` §4.4
//! and §4.7, served by v3 through the CM-1 command seam and the view seam.
//!
//! Written before the implementation by the test-writer claude/07d3765f, on the BE lane controller
//! claude/ad05bf8d's order (rule A; the ticket author claude/89458e53 is unreachable). Stacked on #623's
//! tests-first branch `feat/v3-cash-branch-day-v3`. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! **What is pinned: the lane controller's rulings on #624 (issuecomment of 2026-10-11T02:32Z).**
//! - **G2a.** Every CA-8 command and view inherits `EntryPath::Rpc`. The refusals are the seam's codes:
//!   nobody signed in → `signIn`; a permission not held → `roleNotAllowedHere`; another branch →
//!   `outOfScope`.
//! - **G2.** Each command's and view's permission is an area row in `xc-auth/.../policy/areas/cash.rs`:
//!   `countSubmit` `AnyOf[ShiftOperate, CashManage]`; `countVerify` `ShiftCloseCertify`; `handoverStart`
//!   `ShiftOperate`; `transferCreate`/`transferSubmit`/`transferDispatch` `CashManage`; `transferReceive`
//!   `AnyOf[CashManage, ShiftOperate]`; `transferReconcile` `FinanceReconcile`; views `my_shift` and
//!   `handovers_pending` `ShiftOperate`, `cash_counts` `AnyOf[ShiftOperate, ShiftCloseCertify,
//!   CashManage]`, `cash_transfers` `CashManage`.
//! - **G1.** The bodies are camelCase, with the path `{id}` merged in as `id`, `branch` the optional body field
//!   the seam reads, and `Idempotency-Key` a header only. `qty` and `qtyReceived` are decimal **strings** of a
//!   whole number ≥ 0. A JSON number is not the Input type, so the seam refuses it `invalidRequest`
//!   (`http/v3/command.rs:239-249`); a string that is not a whole number ≥ 0 is refused `validationFailed`. A command
//!   answers `{id}`, plus `status` where the record has one. View rows are the design's records camelCased.
//!
//! **Not here, by the same rulings:** `transferPolicySave` (held for a new permission ticket, G4); `clock`,
//! `dayStartService`, `dayCutoff`, `dayTarget`, `holdCreate`, `holdRelease` and `draftBegin` (moved to CA-8b
//! #1274, G5). `handoverAccept`/`handoverReject` are CA-8's (G3, design PR 1269): `POST /api/handovers/{id}/
//! accept|reject {id}`, row `ShiftOperate`, only the hand-over's receiver decides it, and there is no reason field.
//! `transferReceive`'s `Locks::Pair` (CM-2 #704) is not observable from outside the seam and is not pinned.
//!
//! **Choices of this file**, each marked where used:
//! - `transferReceive` carries the destination branch as `branch`, because its receiver is scoped there (D:§3.1
//!   "a person of the destination", `CashManage` on the destination branch).
//! - A refusal is pinned by its code, not its HTTP status (`PolicyError::status()` decides that).
//! - The approval between submit and dispatch is made with `TransferFlow::decide`. The decide route is PR
//!   570's, not CA-8's.
//!
//! The callers are Blueprint `access::Actor`s (roles scoped to a branch, or organisation-wide), backed by
//! real `users` rows, handed to the seam through `SessionReader::callers`, as `v3_cash_limits_test.rs` does.
//! Expectations about who may call are computed from `RolePermissions`, never from a role name; roles
//! appear only to *make* an actor. Needs `TEST_DATABASE_URL`; nothing skips. Each test works in a schema of
//! its own.

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
use xc_auth::domain::access::{Actor as User, Permission, Role, RolePermissions, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_cash::application::transfer::TransferFlow;
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
/// Any id: the refusals below happen before the id is looked up.
/// The wire code of the receiver rule's refusal (`CashError::NotReceiver`, the person rule): `outOfScope`, 403
/// (#624 Q-B, ruled by the lane controller; D §10 `:723`).
const NOT_THE_RECEIVER: &str = "outOfScope";
/// The wire code of a decision on a hand-over that is no longer PENDING (`HandoverError::NotPending`):
/// `staleVersion`, 409 (#624 Q-B, ruled).
const NOT_PENDING: &str = "staleVersion";
/// The `cash_counts` `status` filter values, derived from `verified_by` (#624 G1 and Q-A rulings; D:179).
const WAITING: &str = "WAITING";
const VERIFIED: &str = "VERIFIED";
const ANY_ID: &str = "00000000-0000-0000-0000-00000000c0a8";

/// The Blueprint staff roles, used only to *make* actors (no expectation names one).
const STAFF_ROLES: [Role; 8] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
    Role::VerificationReviewer,
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

// ------------------------------------------------------------ the commands and views in CA-8

/// An area row: the command or view is allowed to a holder of any of these permissions.
type Row = &'static [Permission];

/// The ten CA-8 commands (G1, G2, G3): name, method, route with `{id}` filled, area row.
fn commands() -> Vec<(&'static str, Method, String, Row)> {
    use Permission::*;
    let post = Method::POST;
    vec![
        (
            "countSubmit",
            post.clone(),
            "/api/cash-counts".into(),
            &[ShiftOperate, CashManage],
        ),
        (
            "countVerify",
            post.clone(),
            format!("/api/cash-counts/{ANY_ID}/verify"),
            &[ShiftCloseCertify],
        ),
        (
            "handoverStart",
            post.clone(),
            "/api/handovers".into(),
            &[ShiftOperate],
        ),
        (
            "handoverAccept",
            post.clone(),
            format!("/api/handovers/{ANY_ID}/accept"),
            &[ShiftOperate],
        ),
        (
            "handoverReject",
            post.clone(),
            format!("/api/handovers/{ANY_ID}/reject"),
            &[ShiftOperate],
        ),
        (
            "transferCreate",
            post.clone(),
            "/api/cash-transfers".into(),
            &[CashManage],
        ),
        (
            "transferSubmit",
            post.clone(),
            format!("/api/cash-transfers/{ANY_ID}/submit"),
            &[CashManage],
        ),
        (
            "transferDispatch",
            post.clone(),
            format!("/api/cash-transfers/{ANY_ID}/dispatch"),
            &[CashManage],
        ),
        (
            "transferReceive",
            post.clone(),
            format!("/api/cash-transfers/{ANY_ID}/receive"),
            &[CashManage, ShiftOperate],
        ),
        (
            "transferReconcile",
            post,
            format!("/api/cash-transfers/{ANY_ID}/reconcile"),
            &[FinanceReconcile],
        ),
    ]
}

/// The four CA-8 views (§4.7, G2) and their area rows.
fn views() -> Vec<(&'static str, Row)> {
    use Permission::*;
    vec![
        (
            "cash_counts",
            &[ShiftOperate, ShiftCloseCertify, CashManage],
        ),
        ("cash_transfers", &[CashManage]),
        ("handovers_pending", &[ShiftOperate]),
        ("my_shift", &[ShiftOperate]),
    ]
}

fn holds_any(role: Role, row: Row) -> bool {
    row.iter().any(|p| RolePermissions::of(role).contains(p))
}

/// A staff role that holds at least one permission of `row`.
fn a_holder(row: Row) -> Role {
    *STAFF_ROLES
        .iter()
        .find(|r| holds_any(**r, row))
        .unwrap_or_else(|| panic!("no staff role holds any of {row:?}"))
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
            // USD 500.00 in each safe, for the transfer flow
            sqlx::query(
                "INSERT INTO ops_state (branch_id, enabled, phase, safe) \
                 VALUES ($1, true, 'OPEN', jsonb_build_object('USD', 50000::bigint))",
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

// ------------------------------------------------------------ routes and refusals (G2, G2a)

/// Every command route is served on `EntryPath::Rpc`: nobody signed in gets the seam's `signIn`, never the
/// unknown-route 404 or a 405.
#[actix_web::test]
async fn nobody_signed_in_is_asked_to_sign_in_by_every_cash_command() {
    let room = Room::start().await;
    let mut wrong = Vec::new();
    for (name, method, uri, _) in commands() {
        let a = call(
            &room,
            None,
            method.clone(),
            &uri,
            Some(json!({ "branch": B01 })),
            Key::Fresh,
        )
        .await;
        if code(&a) != "signIn" {
            wrong.push(format!(
                "{name} {method} {uri} → {} {:.120}",
                a.status, a.text
            ));
        }
    }
    assert!(wrong.is_empty(), "not refused with signIn: {wrong:#?}");
    room.finish().await;
}

#[actix_web::test]
async fn nobody_signed_in_is_asked_to_sign_in_by_every_cash_view() {
    let room = Room::start().await;
    for (name, _) in views() {
        let a = call(
            &room,
            None,
            Method::GET,
            &format!("/api/v3/views/{name}?branch={B01}"),
            None,
            Key::Fresh,
        )
        .await;
        assert_eq!(
            code(&a),
            "signIn",
            "view {name}: {} {:.200}",
            a.status,
            a.text
        );
    }
    room.finish().await;
}

/// Exactly the holders of a command's area row pass its role check: every staff role holding none of the row is
/// refused `roleNotAllowedHere`, and every role holding one of it is not (whatever else it is then refused for).
#[actix_web::test]
async fn exactly_the_holders_of_each_commands_row_pass_its_role_check() {
    let room = Room::start().await;
    for (name, method, uri, row) in commands() {
        for role in STAFF_ROLES {
            let who = room.person(role, Some(B01)).await;
            let a = call(
                &room,
                Some(&who),
                method.clone(),
                &uri,
                Some(json!({ "branch": B01 })),
                Key::Fresh,
            )
            .await;
            if holds_any(role, row) {
                assert_ne!(
                    code(&a),
                    "roleNotAllowedHere",
                    "{name} as {role:?}, a holder of {row:?}: {}",
                    a.text
                );
            } else {
                assert_eq!(
                    code(&a),
                    "roleNotAllowedHere",
                    "{name} as {role:?}, holding none of {row:?}: {}",
                    a.text
                );
            }
        }
    }
    room.finish().await;
}

/// Exactly the holders of a view's area row may read it.
#[actix_web::test]
async fn exactly_the_holders_of_each_views_row_may_read_it() {
    let room = Room::start().await;
    for (name, row) in views() {
        for role in STAFF_ROLES {
            let who = room.person(role, Some(B01)).await;
            let a = view(&room, &who, name, "").await;
            if holds_any(role, row) {
                assert_eq!(
                    a.status,
                    StatusCode::OK,
                    "view {name} as {role:?}, a holder of {row:?}: {}",
                    a.text
                );
            } else {
                assert_eq!(
                    code(&a),
                    "roleNotAllowedHere",
                    "view {name} as {role:?}, holding none of {row:?}: {}",
                    a.text
                );
            }
        }
    }
    room.finish().await;
}

/// A holder of the row whose role is on another branch is refused `outOfScope` for this branch.
#[actix_web::test]
async fn a_holder_of_another_branch_is_out_of_scope_on_every_cash_command_and_view() {
    let room = Room::start().await;
    for (name, method, uri, row) in commands() {
        let who = room.person(a_holder(row), Some(B09)).await;
        let a = call(
            &room,
            Some(&who),
            method,
            &uri,
            Some(json!({ "branch": B01 })),
            Key::Fresh,
        )
        .await;
        assert_eq!(
            code(&a),
            "outOfScope",
            "{name} for {B01} by a {B09} holder: {}",
            a.text
        );
    }
    for (name, row) in views() {
        let who = room.person(a_holder(row), Some(B09)).await;
        let a = view(&room, &who, name, "").await;
        assert_eq!(
            code(&a),
            "outOfScope",
            "view {name} for {B01} by a {B09} holder: {}",
            a.text
        );
    }
    room.finish().await;
}

/// C8: a holder of the row who sends no `Idempotency-Key` is refused `validationFailed` on `idempotencyKey`.
#[actix_web::test]
async fn every_cash_command_needs_an_idempotency_key() {
    let room = Room::start().await;
    for (name, method, uri, row) in commands() {
        let who = room.person(a_holder(row), Some(B01)).await;
        let a = call(
            &room,
            Some(&who),
            method,
            &uri,
            Some(json!({ "branch": B01 })),
            Key::Missing,
        )
        .await;
        assert_eq!(
            code(&a),
            "validationFailed",
            "{name} with no key: {}",
            a.text
        );
        assert_eq!(
            a.body["error"]["field"], "idempotencyKey",
            "{name}: {}",
            a.text
        );
    }
    room.finish().await;
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

#[actix_web::test]
async fn a_count_submitted_and_verified_over_http_opens_the_tellers_shift() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;

    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        open_count_body(&room).await,
    )
    .await;
    let count = id_of(&a, "countSubmit");
    assert_eq!(
        room.text(&format!(
            "SELECT counted_by::text FROM cash_counts WHERE id = '{count}'::uuid"
        ))
        .await
        .as_deref(),
        Some(teller.id.as_str()),
        "the count is the caller's"
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM cash_count_lines WHERE count_id = '{count}'::uuid"
        ))
        .await,
        2
    );

    let a = post(
        &room,
        &manager,
        &format!("/api/cash-counts/{count}/verify"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(
        id_of(&a, "countVerify"),
        count,
        "countVerify answers the count's id"
    );
    assert_eq!(
        a.body["status"], "VERIFIED",
        "within the limit countVerify answers VERIFIED (D §4.9): {}",
        a.text
    );
    assert!(
        a.body.get("approvalRequestId").is_none_or(Value::is_null),
        "a verified count carries no approval request: {}",
        a.text
    );
    assert_eq!(
        room.text(&format!(
            "SELECT verified_by::text FROM cash_counts WHERE id = '{count}'::uuid"
        ))
        .await
        .as_deref(),
        Some(manager.id.as_str())
    );
    assert_eq!(
        room.scalar(&format!(
            "SELECT count(*) FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            teller.id
        ))
        .await,
        1,
        "the verified OPEN count opens the teller's shift (D:§4.2)"
    );
    room.finish().await;
}

#[actix_web::test]
async fn qty_is_a_string_of_a_whole_number_and_anything_else_is_refused() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let thb1000 = room.denomination("THB", "1000").await;
    for (qty, what, expected) in [
        // not the Input type (qty: string): the seam's refusal, `http/v3/command.rs:239-249`
        (json!(2), "a JSON number", "invalidRequest"),
        (json!("1.5"), "a fraction", "validationFailed"),
        (json!("-1"), "a negative", "validationFailed"),
        (json!("two"), "not a number", "validationFailed"),
    ] {
        let a = post(
            &room,
            &teller,
            "/api/cash-counts",
            json!({
                "branch": B01,
                "kind": "OPEN",
                "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": qty }]
            }),
        )
        .await;
        assert_eq!(code(&a), expected, "qty {what}: {}", a.text);
    }
    assert_eq!(
        room.scalar("SELECT count(*) FROM cash_counts").await,
        0,
        "nothing is written"
    );

    let a = post(
        &room,
        &teller,
        "/api/cash-counts",
        json!({
            "branch": B01,
            "kind": "OPEN",
            "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "0" }]
        }),
    )
    .await;
    id_of(&a, "qty \"0\" is a whole number ≥ 0");
    room.finish().await;
}

#[actix_web::test]
async fn the_same_key_replays_the_same_count_and_another_body_under_it_is_a_conflict() {
    let room = Room::start().await;
    let teller = room.person(Role::Teller, Some(B01)).await;
    let key = unique("replay");
    let body = open_count_body(&room).await;

    let first = call(
        &room,
        Some(&teller),
        Method::POST,
        "/api/cash-counts",
        Some(body.clone()),
        Key::This(&key),
    )
    .await;
    let again = call(
        &room,
        Some(&teller),
        Method::POST,
        "/api/cash-counts",
        Some(body.clone()),
        Key::This(&key),
    )
    .await;
    assert_eq!(id_of(&again, "the replay"), id_of(&first, "the first"));
    assert_eq!(
        room.scalar("SELECT count(*) FROM cash_counts").await,
        1,
        "one count"
    );

    let mut other = body;
    other["lines"][0]["qty"] = json!("9");
    let a = call(
        &room,
        Some(&teller),
        Method::POST,
        "/api/cash-counts",
        Some(other),
        Key::This(&key),
    )
    .await;
    assert_eq!(code(&a), "idempotencyConflict", "{}", a.text);
    assert_eq!(
        room.scalar("SELECT count(*) FROM cash_counts").await,
        1,
        "still one count"
    );
    room.finish().await;
}

// ------------------------------------------------------------ hand-over (G1)

#[actix_web::test]
async fn a_handover_started_over_http_is_pending_for_its_receiver() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let thb1000 = room.denomination("THB", "1000").await;

    let a = post(
        &room,
        &teller,
        "/api/handovers",
        json!({
            "branch": B01,
            "fromShiftId": shift,
            "receiverId": receiver.id,
            "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
        }),
    )
    .await;
    let handover = id_of(&a, "handoverStart");
    assert_eq!(a.body["status"], "PENDING", "{}", a.text);
    assert_eq!(
        room.text(&format!(
            "SELECT receiver_id::text FROM handovers WHERE id = '{handover}'::uuid"
        ))
        .await
        .as_deref(),
        Some(receiver.id.as_str())
    );

    let pending = data(
        &view(&room, &receiver, "handovers_pending", "").await,
        "handovers_pending",
    );
    let rows = pending
        .as_array()
        .unwrap_or_else(|| panic!("a list of hand-overs: {pending}"));
    assert!(
        rows.iter().any(|r| r["id"] == json!(handover)),
        "the receiver sees the pending hand-over: {pending}"
    );
    room.finish().await;
}

/// A hand-over from an opened shift to a second teller: (outgoing teller, receiver, hand-over id).
async fn started_handover(room: &Room) -> (Who, Who, String) {
    let (teller, _, shift) = opened_shift(room).await;
    let receiver = room.person(Role::Teller, Some(B01)).await;
    let thb1000 = room.denomination("THB", "1000").await;
    let a = post(
        room,
        &teller,
        "/api/handovers",
        json!({
            "branch": B01,
            "fromShiftId": shift,
            "receiverId": receiver.id,
            "lines": [{ "currency": "THB", "denominationId": thb1000, "qty": "2" }]
        }),
    )
    .await;
    let id = id_of(&a, "handoverStart");
    (teller, receiver, id)
}

async fn handover_status(room: &Room, id: &str) -> Option<String> {
    room.text(&format!(
        "SELECT status FROM handovers WHERE id = '{id}'::uuid"
    ))
    .await
}

#[actix_web::test]
async fn the_receiver_accepts_the_handover_and_it_leaves_the_pending_list() {
    let room = Room::start().await;
    let (_, receiver, id) = started_handover(&room).await;
    let a = post(
        &room,
        &receiver,
        &format!("/api/handovers/{id}/accept"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(id_of(&a, "handoverAccept"), id);
    assert_eq!(a.body["status"], "ACCEPTED", "{}", a.text);
    assert_eq!(
        handover_status(&room, &id).await.as_deref(),
        Some("ACCEPTED")
    );
    let pending = data(
        &view(&room, &receiver, "handovers_pending", "").await,
        "handovers_pending",
    );
    assert!(
        !pending.to_string().contains(&id),
        "an accepted hand-over is no longer pending: {pending}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn the_receiver_rejects_the_handover_with_no_reason_field() {
    let room = Room::start().await;
    let (_, receiver, id) = started_handover(&room).await;
    let a = post(
        &room,
        &receiver,
        &format!("/api/handovers/{id}/reject"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(id_of(&a, "handoverReject"), id);
    assert_eq!(a.body["status"], "REJECTED", "{}", a.text);
    assert_eq!(
        handover_status(&room, &id).await.as_deref(),
        Some("REJECTED")
    );
    room.finish().await;
}

/// The receiver rule: a holder of `ShiftOperate` who is not the hand-over's receiver (the outgoing teller)
/// can neither accept nor reject it, and it stays PENDING. The refusal is pinned to its wire code
/// ([`NOT_THE_RECEIVER`]), so a panic or a `500` does not pass.
#[actix_web::test]
async fn only_the_receiver_decides_the_handover() {
    let room = Room::start().await;
    let (teller, _, id) = started_handover(&room).await;
    for step in ["accept", "reject"] {
        let a = post(
            &room,
            &teller,
            &format!("/api/handovers/{id}/{step}"),
            json!({ "branch": B01 }),
        )
        .await;
        assert_eq!(
            (a.status, code(&a).as_str()),
            (StatusCode::FORBIDDEN, NOT_THE_RECEIVER),
            "{step} by the outgoing teller is the receiver rule's refusal: {}",
            a.text
        );
    }
    assert_eq!(
        handover_status(&room, &id).await.as_deref(),
        Some("PENDING")
    );
    room.finish().await;
}

// ------------------------------------------------------------ transfers (G1)

#[actix_web::test]
async fn a_branch_transfer_walks_create_submit_dispatch_receive_reconcile_over_http() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let receiver = room.person(Role::BranchManager, Some(B09)).await;
    let finance = room.person(Role::Finance, None).await;
    let executive = room.person(Role::Executive, None).await;
    let usd50 = room.denomination("USD", "50").await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '300.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(&executive.id)
    .execute(&room.pool)
    .await
    .expect("a transfer policy version");

    let a = post(
        &room,
        &creator,
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
    let id = id_of(&a, "transferCreate");
    assert_eq!(a.body["status"], "DRAFT", "{}", a.text);

    let a = post(
        &room,
        &creator,
        &format!("/api/cash-transfers/{id}/submit"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(id_of(&a, "transferSubmit"), id);
    assert_eq!(
        a.body["status"], "DRAFT",
        "submitting asks the approval core; it stays DRAFT: {}",
        a.text
    );

    // the approval decision is PR 570's route, not CA-8's (choice): made in-process
    let approver_user = user_of(&approver.spec).expect("the approver");
    TransferFlow::pg(room.pool.clone())
        .decide(&approver_user, &id, Outcome::Approved, "ok", Utc::now())
        .await
        .expect("a branch manager of the source approves within the limit");

    let a = post(
        &room,
        &creator,
        &format!("/api/cash-transfers/{id}/dispatch"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(a.body["status"], "DISPATCHED", "{}", a.text);

    // choice: the receiver names the destination branch, where their role is
    let a = post(
        &room,
        &receiver,
        &format!("/api/cash-transfers/{id}/receive"),
        json!({
            "branch": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qtyReceived": "2" }]
        }),
    )
    .await;
    assert_eq!(a.body["status"], "RECEIVED", "{}", a.text);

    let a = post(
        &room,
        &finance,
        &format!("/api/cash-transfers/{id}/reconcile"),
        json!({ "branch": B01, "note": "checked" }),
    )
    .await;
    assert_eq!(a.body["status"], "RECONCILED", "{}", a.text);
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM cash_transfers WHERE id = '{id}'::uuid"
        ))
        .await
        .as_deref(),
        Some("RECONCILED")
    );

    let listed = data(
        &view(&room, &creator, "cash_transfers", "").await,
        "cash_transfers",
    );
    let rows = listed
        .as_array()
        .unwrap_or_else(|| panic!("a list of transfers: {listed}"));
    let row = rows
        .iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or_else(|| panic!("the transfer is listed: {listed}"));
    assert_eq!(row["status"], "RECONCILED", "{row}");
    assert_eq!(row["kind"], "BRANCH", "{row}");
    room.finish().await;
}

/// The lane controller's G9 ruling on #624: `transferReceive` locks only the session branch
/// (`Locks::One`), so the session branch must be the transfer's destination. A caller who holds the row on
/// another branch (here a branch manager of the *source*, in scope there) is refused `outOfScope`, and the
/// transfer stays DISPATCHED.
#[actix_web::test]
async fn transfer_receive_from_a_session_branch_that_is_not_the_destination_is_out_of_scope() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let source_manager = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    let usd50 = room.denomination("USD", "50").await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '300.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(&executive.id)
    .execute(&room.pool)
    .await
    .expect("a transfer policy version");
    let id = id_of(
        &post(
            &room,
            &creator,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "BRANCH",
                "toBranchId": B09,
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "2" }],
                "note": "to B09"
            }),
        )
        .await,
        "transferCreate",
    );
    ok(
        &post(
            &room,
            &creator,
            &format!("/api/cash-transfers/{id}/submit"),
            json!({ "branch": B01 }),
        )
        .await,
        "transferSubmit",
    );
    TransferFlow::pg(room.pool.clone())
        .decide(
            &user_of(&approver.spec).expect("the approver"),
            &id,
            Outcome::Approved,
            "ok",
            Utc::now(),
        )
        .await
        .expect("approved");
    ok(
        &post(
            &room,
            &creator,
            &format!("/api/cash-transfers/{id}/dispatch"),
            json!({ "branch": B01 }),
        )
        .await,
        "transferDispatch",
    );

    let a = post(
        &room,
        &source_manager,
        &format!("/api/cash-transfers/{id}/receive"),
        json!({
            "branch": B01,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qtyReceived": "2" }]
        }),
    )
    .await;
    assert_eq!(
        code(&a),
        "outOfScope",
        "receive with the source as the session branch: {}",
        a.text
    );
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM cash_transfers WHERE id = '{id}'::uuid"
        ))
        .await
        .as_deref(),
        Some("DISPATCHED"),
        "nothing is received"
    );
    room.finish().await;
}

#[actix_web::test]
async fn qty_received_is_a_string_too() {
    let room = Room::start().await;
    let receiver = room.person(Role::BranchManager, Some(B09)).await;
    let usd50 = room.denomination("USD", "50").await;
    let a = post(
        &room,
        &receiver,
        &format!("/api/cash-transfers/{ANY_ID}/receive"),
        json!({
            "branch": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qtyReceived": 2 }]
        }),
    )
    .await;
    assert_eq!(
        code(&a),
        "invalidRequest",
        "qtyReceived as a JSON number: {}",
        a.text
    );
    room.finish().await;
}

// ------------------------------------------------------------ views (§4.7)

#[actix_web::test]
async fn my_shift_is_the_callers_open_shift() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let answer = data(&view(&room, &teller, "my_shift", "").await, "my_shift");
    assert!(
        answer.to_string().contains(&shift),
        "my_shift names the caller's open shift {shift}: {answer}"
    );

    let other = room.person(Role::Teller, Some(B01)).await;
    let answer = data(
        &view(&room, &other, "my_shift", "").await,
        "my_shift of a teller with no shift",
    );
    assert!(
        !answer.to_string().contains(&shift),
        "another teller does not see this shift as theirs: {answer}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn cash_counts_lists_the_branchs_counts_with_who_counted_and_verified() {
    let room = Room::start().await;
    let (teller, count, _) = opened_shift(&room).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let listed = data(
        &view(&room, &manager, "cash_counts", "").await,
        "cash_counts",
    );
    let rows = listed
        .as_array()
        .unwrap_or_else(|| panic!("a list of counts: {listed}"));
    let row = rows
        .iter()
        .find(|r| r["id"] == json!(count))
        .unwrap_or_else(|| panic!("the count is listed: {listed}"));
    assert_eq!(row["kind"], "OPEN", "{row}");
    assert_eq!(row["countedBy"], json!(teller.id), "{row}");
    assert!(
        row["verifiedBy"].is_string(),
        "the verified count names its verifier: {row}"
    );
    room.finish().await;
}

// ------------------------------------------------------------ dell-claude-5's gaps (#624 issuecomment-6105924497)

/// The ids of a view page, in the order the view answered them.
fn ids(page: &Value) -> Vec<String> {
    page.as_array()
        .unwrap_or_else(|| panic!("a page of rows: {page}"))
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .unwrap_or_else(|| panic!("a row with an id: {r}"))
                .to_owned()
        })
        .collect()
}

/// Gap 1, D §9.1: `countVerify` with a difference over the branch's limit answers 200 `AWAITING_FINANCE` with
/// the approval request's id, leaves exactly one PENDING request, and the count stays unverified.
#[actix_web::test]
async fn count_verify_over_the_limit_answers_awaiting_finance_with_one_pending_request() {
    let room = Room::start().await;
    let (teller, _, shift) = opened_shift(&room).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    sqlx::query(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
         VALUES ($1::uuid, $2, 50.00, 'test', now())",
    )
    .bind(ORG)
    .bind(B01)
    .execute(&room.pool)
    .await
    .expect("a count difference limit of THB 50");
    // the shift expects THB 2,300; this CLOSE count holds THB 2,200: 100 short, over the limit
    let close = id_of(
        &post(
            &room,
            &teller,
            "/api/cash-counts",
            json!({
                "branch": B01,
                "kind": "CLOSE",
                "shiftId": shift,
                "lines": [
                    { "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "2" },
                    { "currency": "THB", "denominationId": room.denomination("THB", "100").await, "qty": "2" }
                ]
            }),
        )
        .await,
        "the short CLOSE count",
    );

    let a = post(
        &room,
        &manager,
        &format!("/api/cash-counts/{close}/verify"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(id_of(&a, "countVerify over the limit is a 200"), close);
    assert_eq!(a.body["status"], "AWAITING_FINANCE", "{}", a.text);
    let request = a.body["approvalRequestId"]
        .as_str()
        .unwrap_or_else(|| panic!("AWAITING_FINANCE carries approvalRequestId: {}", a.text))
        .to_owned();
    assert_eq!(
        room.scalar("SELECT count(*) FROM approval_requests WHERE status = 'PENDING'")
            .await,
        1,
        "exactly one PENDING approval request"
    );
    assert_eq!(
        room.text(&format!(
            "SELECT type FROM approval_requests WHERE id::text = '{request}' AND status = 'PENDING'"
        ))
        .await
        .as_deref(),
        Some("COUNT_DIFFERENCE"),
        "approvalRequestId names that request"
    );
    assert_eq!(
        room.text(&format!(
            "SELECT verified_by::text FROM cash_counts WHERE id = '{close}'::uuid"
        ))
        .await,
        None,
        "the count waits for FINANCE, unverified"
    );
    room.finish().await;
}

/// Gap 2, D §4.7 and the parent view contract (`xchange-v3-web.md:168`): `cash_counts` pages newest first by
/// `limit` and `before`, and filters by `shift` and by `status` (waiting / verified) in the server.
#[actix_web::test]
async fn cash_counts_pages_newest_first_and_filters_by_shift_and_status() {
    let room = Room::start().await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let (teller, verified, shift) = opened_shift(&room).await;
    let mut waiting_open = Vec::new();
    for _ in 0..3 {
        let other = room.person(Role::Teller, Some(B01)).await;
        waiting_open.push(id_of(
            &post(
                &room,
                &other,
                "/api/cash-counts",
                open_count_body(&room).await,
            )
            .await,
            "another teller's OPEN count",
        ));
    }
    let close = id_of(
        &post(
            &room,
            &teller,
            "/api/cash-counts",
            json!({
                "branch": B01,
                "kind": "CLOSE",
                "shiftId": shift,
                "lines": [
                    { "currency": "THB", "denominationId": room.denomination("THB", "1000").await, "qty": "2" },
                    { "currency": "THB", "denominationId": room.denomination("THB", "100").await, "qty": "3" }
                ]
            }),
        )
        .await,
        "the teller's CLOSE count",
    );
    // created in this order: verified, waiting_open[0..3], close
    let newest_first = vec![
        close.clone(),
        waiting_open[2].clone(),
        waiting_open[1].clone(),
        waiting_open[0].clone(),
        verified.clone(),
    ];
    let page = |q: String| {
        let room = &room;
        let manager = &manager;
        async move { ids(&data(&view(room, manager, "cash_counts", &q).await, &q)) }
    };

    assert_eq!(
        page(String::new()).await,
        newest_first,
        "every count, newest first"
    );
    assert_eq!(page("&limit=2".into()).await, newest_first[0..2]);
    assert_eq!(
        page(format!("&limit=2&before={}", newest_first[1])).await,
        newest_first[2..4]
    );
    assert_eq!(
        page(format!("&limit=2&before={}", newest_first[3])).await,
        newest_first[4..5]
    );
    assert!(
        page(format!("&limit=2&before={}", newest_first[4]))
            .await
            .is_empty(),
        "nothing older than the oldest"
    );

    assert_eq!(
        page(format!("&status={WAITING}")).await,
        newest_first[0..4],
        "waiting = not verified yet"
    );
    assert_eq!(
        page(format!("&status={VERIFIED}")).await,
        vec![verified.clone()]
    );
    let of_shift = page(format!("&shift={shift}")).await;
    assert!(
        of_shift.contains(&close),
        "the shift's CLOSE count: {of_shift:?}"
    );
    for other in &waiting_open {
        assert!(
            !of_shift.contains(other),
            "another teller's count is not of this shift: {of_shift:?}"
        );
    }
    assert_eq!(
        page(format!("&shift={shift}&status={WAITING}")).await,
        vec![close.clone()],
        "the filters combine"
    );
    assert_eq!(
        page(format!(
            "&status={WAITING}&limit=1&before={}",
            newest_first[1]
        ))
        .await,
        newest_first[2..3],
        "paging stays inside the filter"
    );
    room.finish().await;
}

/// Gap 2, D §4.7: `cash_transfers` pages newest first by `limit` and `before`, and filters by `status` and by
/// `kind` (the table's values, D:21-23) in the server.
#[actix_web::test]
async fn cash_transfers_pages_newest_first_and_filters_by_status_and_kind() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    let usd50 = room.denomination("USD", "50").await;
    sqlx::query(
        "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
         VALUES ('v1', $1::uuid, jsonb_build_object('USD', '300.00'), $2::uuid)",
    )
    .bind(ORG)
    .bind(&executive.id)
    .execute(&room.pool)
    .await
    .expect("a transfer policy version");
    let mut made = Vec::new();
    for n in 0..3 {
        made.push(id_of(
            &post(
                &room,
                &creator,
                "/api/cash-transfers",
                json!({
                    "branch": B01,
                    "kind": "BRANCH",
                    "toBranchId": B09,
                    "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                    "note": format!("branch {n}")
                }),
            )
            .await,
            "a BRANCH draft",
        ));
    }
    made.push(id_of(
        &post(
            &room,
            &creator,
            "/api/cash-transfers",
            json!({
                "branch": B01,
                "kind": "EXTERNAL_OUT",
                "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "1" }],
                "note": "out"
            }),
        )
        .await,
        "an EXTERNAL_OUT draft",
    ));
    // the first BRANCH draft is submitted and approved, so its status is APPROVED
    ok(
        &post(
            &room,
            &creator,
            &format!("/api/cash-transfers/{}/submit", made[0]),
            json!({ "branch": B01 }),
        )
        .await,
        "transferSubmit",
    );
    TransferFlow::pg(room.pool.clone())
        .decide(
            &user_of(&approver.spec).expect("the approver"),
            &made[0],
            Outcome::Approved,
            "ok",
            Utc::now(),
        )
        .await
        .expect("approved within the limit");
    let newest_first: Vec<String> = made.iter().rev().cloned().collect();
    let page = |q: String| {
        let room = &room;
        let creator = &creator;
        async move { ids(&data(&view(room, creator, "cash_transfers", &q).await, &q)) }
    };

    assert_eq!(
        page(String::new()).await,
        newest_first,
        "every transfer, newest first"
    );
    assert_eq!(page("&limit=3".into()).await, newest_first[0..3]);
    assert_eq!(
        page(format!("&limit=3&before={}", newest_first[2])).await,
        newest_first[3..4]
    );
    assert!(
        page(format!("&limit=3&before={}", newest_first[3]))
            .await
            .is_empty(),
        "nothing older than the oldest"
    );

    assert_eq!(
        page("&kind=EXTERNAL_OUT".into()).await,
        vec![made[3].clone()]
    );
    assert_eq!(page("&status=APPROVED".into()).await, vec![made[0].clone()]);
    assert_eq!(
        page("&status=DRAFT&kind=BRANCH".into()).await,
        vec![made[2].clone(), made[1].clone()],
        "the filters combine"
    );
    assert_eq!(
        page(format!("&status=DRAFT&limit=1&before={}", made[2])).await,
        vec![made[1].clone()],
        "paging stays inside the filter"
    );
    room.finish().await;
}

/// Gap 3, D §9.1: a hand-over that is no longer PENDING is refused to its own receiver, whichever decision
/// came first, and its status does not move.
#[actix_web::test]
async fn a_decided_handover_is_refused_and_keeps_its_status() {
    let room = Room::start().await;
    for (first, expected) in [("accept", "ACCEPTED"), ("reject", "REJECTED")] {
        let (_, receiver, id) = started_handover(&room).await;
        ok(
            &post(
                &room,
                &receiver,
                &format!("/api/handovers/{id}/{first}"),
                json!({ "branch": B01 }),
            )
            .await,
            first,
        );
        for again in ["accept", "reject"] {
            let a = post(
                &room,
                &receiver,
                &format!("/api/handovers/{id}/{again}"),
                json!({ "branch": B01 }),
            )
            .await;
            assert_eq!(
                (a.status, code(&a).as_str()),
                (StatusCode::CONFLICT, NOT_PENDING),
                "{again} after {first}: {}",
                a.text
            );
            assert_eq!(
                handover_status(&room, &id).await.as_deref(),
                Some(expected),
                "{again} after {first} leaves it {expected}"
            );
        }
    }
    room.finish().await;
}

/// The lane controller's rulings on #624 (issuecomment-6107796305, amended by -6107853376), added by the
/// implementer claude/dell-claude-4: `denominations` answers `{rows: [{id, currency, faceValue, groupCode}]}`, only
/// the countable rows (no catch-all `face_value NULL` row), ordered by `currency, face_value, id`, and inherits
/// `cash_counts`'s row, so a role without it is refused.
#[actix_web::test]
async fn denominations_lists_face_values_in_order_and_refuses_a_role_without_the_cash_counts_row() {
    let room = Room::start().await;
    // a currency with only its catch-all row; its face values are added in reverse order, so `id` order is not the
    // face-value order
    let currency = room
        .text(
            "SELECT c.code::text FROM currencies c WHERE NOT EXISTS (SELECT 1 FROM denominations d \
             WHERE d.currency_code = c.code AND d.face_value IS NOT NULL) ORDER BY c.code LIMIT 1",
        )
        .await
        .expect("a currency with no face value yet");
    for face in ["100", "20"] {
        sqlx::query(
            "INSERT INTO denominations (currency_code, face_value, group_code) VALUES ($1, $2::numeric, 'ALL')",
        )
        .bind(&currency)
        .bind(face)
        .execute(&room.pool)
        .await
        .expect("a face value");
    }
    let (_, row) = views()
        .into_iter()
        .find(|(n, _)| *n == "cash_counts")
        .expect("cash_counts' row");

    let counter = room.person(a_holder(row), Some(B01)).await;
    let answer = data(
        &view(&room, &counter, "denominations", "").await,
        "denominations",
    );
    let rows = answer["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{{rows: [...]}}: {answer}"));
    let of_currency: Vec<(Value, Value)> = rows
        .iter()
        .filter(|r| r["currency"] == json!(currency))
        .map(|r| (r["faceValue"].clone(), r["groupCode"].clone()))
        .collect();
    assert_eq!(
        of_currency,
        vec![
            (json!("20.0000"), json!("ALL")),
            (json!("100.0000"), json!("ALL"))
        ],
        "{currency}: by face value, not by id, and no catch-all row"
    );
    for r in rows {
        assert!(r["id"].is_i64(), "id is a number: {r}");
        assert!(r["faceValue"].is_string(), "only countable rows: {r}");
    }
    // the whole answer is in `currency, face_value NULLS FIRST, id` order
    let key = |r: &Value| {
        let face = rust_decimal::Decimal::from_str(r["faceValue"].as_str().unwrap_or(""))
            .expect("a decimal face value");
        (
            r["currency"].as_str().unwrap_or("").to_owned(),
            face,
            r["id"].as_i64().unwrap_or(0),
        )
    };
    let keys: Vec<_> = rows.iter().map(key).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "rows ordered by currency, face value, id");

    let outsider = STAFF_ROLES
        .into_iter()
        .find(|r| !holds_any(*r, row))
        .expect("a role without cash_counts' row");
    let who = room.person(outsider, Some(B01)).await;
    let a = view(&room, &who, "denominations", "").await;
    assert!(a.status.is_client_error(), "{outsider:?}: {}", a.text);
    assert_eq!(code(&a), "roleNotAllowedHere", "{outsider:?}: {}", a.text);
    room.finish().await;
}

/// The lane controller's ruling on #624 (issuecomment-6107853376), added by the implementer claude/dell-claude-4:
/// `handover_receivers` lists the active users of the session branch who hold `ShiftOperate` there, the caller
/// excluded, as `{rows: [{userId, name}]}` ordered by name and user id; it is authorised like `handoverStart`.
#[actix_web::test]
async fn handover_receivers_lists_the_branchs_shift_operators_but_the_caller() {
    let room = Room::start().await;
    let caller = room.person(Role::Teller, Some(B01)).await;
    let colleague = room.person(Role::Teller, Some(B01)).await;
    let manager = room.person(Role::BranchManager, Some(B01)).await;
    let elsewhere = room.person(Role::Teller, Some(B09)).await;
    let finance = room.person(Role::Finance, Some(B01)).await;
    let gone = room.person(Role::Teller, Some(B01)).await;
    sqlx::query("UPDATE users SET status = 'DISABLED' WHERE id = $1::uuid")
        .bind(&gone.id)
        .execute(&room.pool)
        .await
        .expect("disable a teller");
    // another organisation's organisation-wide BRANCH_MANAGER: holds ShiftOperate everywhere in *their* organisation,
    // never in this one (PR 1325 review, issuecomment-6108602501)
    let other_org = "00000000-0000-0000-0000-0000000000f3";
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) VALUES ($1::uuid, 'XC3', 'Asia/Bangkok', now())",
    )
    .bind(other_org)
    .execute(&room.pool)
    .await
    .expect("another organisation");
    let foreign: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'Foreign manager', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(other_org)
    .fetch_one(&room.pool)
    .await
    .expect("a user of another organisation");
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) VALUES ($1::uuid, $2, $3::uuid, NULL)",
    )
    .bind(&foreign)
    .bind(Role::BranchManager.as_str())
    .bind(other_org)
    .execute(&room.pool)
    .await
    .expect("an organisation-wide BRANCH_MANAGER of that organisation");

    let answer = data(
        &view(&room, &caller, "handover_receivers", "").await,
        "handover_receivers",
    );
    let rows = answer["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{{rows: [...]}}: {answer}"));
    let listed: Vec<String> = rows
        .iter()
        .map(|r| r["userId"].as_str().unwrap_or("").to_owned())
        .collect();
    // the expected people, in name then user id order
    let mut expected: Vec<(String, String)> = Vec::new();
    for who in [&colleague, &manager] {
        let name = room
            .text(&format!(
                "SELECT name FROM users WHERE id = '{}'::uuid",
                who.id
            ))
            .await
            .expect("a name");
        expected.push((name, who.id.clone()));
    }
    expected.sort();
    assert_eq!(
        listed,
        expected
            .iter()
            .map(|(_, id)| id.clone())
            .collect::<Vec<_>>(),
        "only this branch's shift operators, by name: {answer}"
    );
    for r in rows {
        let keys: Vec<&String> = r.as_object().expect("a row").keys().collect();
        assert_eq!(keys.len(), 2, "userId and name only: {r}");
        assert!(r["name"].is_string(), "{r}");
    }
    for (who, what) in [
        (&caller, "the caller"),
        (&elsewhere, "another branch's teller"),
        (&finance, "a FINANCE user"),
        (&gone, "a disabled teller"),
    ] {
        assert!(!listed.contains(&who.id), "{what} is not listed: {answer}");
    }
    assert!(
        !listed.contains(&foreign),
        "another organisation's organisation-wide BRANCH_MANAGER is not listed: {answer}"
    );

    let outsider = STAFF_ROLES
        .into_iter()
        .find(|r| !holds_any(*r, &[Permission::ShiftOperate]))
        .expect("a role without ShiftOperate");
    let who = room.person(outsider, Some(B01)).await;
    let a = view(&room, &who, "handover_receivers", "").await;
    assert!(a.status.is_client_error(), "{outsider:?}: {}", a.text);
    assert_eq!(code(&a), "roleNotAllowedHere", "{outsider:?}: {}", a.text);
    room.finish().await;
}

/// The lane controller's rulings on #624 (issuecomment-6107955194, and A on -6107959194), added by the implementer
/// claude/dell-claude-4: `transfer_branches` lists every branch of the session's organisation except the session
/// branch, as `{rows: [{branchId, name}]}` ordered by name and branch id; it is authorised like `transferCreate`.
#[actix_web::test]
async fn transfer_branches_lists_the_organisations_other_branches_by_name() {
    let room = Room::start().await;
    let other_org = "00000000-0000-0000-0000-0000000000f2";
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) VALUES ($1::uuid, 'XC2', 'Asia/Bangkok', now())",
    )
    .bind(other_org)
    .execute(&room.pool)
    .await
    .expect("another organisation");
    // B09 exists already; two more of this organisation, named so that name order is not id order
    for (id, name) in [("XC-B03", "Aaa branch"), ("XC-B02", "Zzz branch")] {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(&room.pool)
            .await
            .expect("a branch of this organisation");
    }
    sqlx::query(
        "INSERT INTO branches (id, name, organization_id) VALUES ('XC-Z99', 'Other org', $1::uuid)",
    )
    .bind(other_org)
    .execute(&room.pool)
    .await
    .expect("a branch of another organisation");

    let manager = room
        .person(a_holder(&[Permission::CashManage]), Some(B01))
        .await;
    let answer = data(
        &view(&room, &manager, "transfer_branches", "").await,
        "transfer_branches",
    );
    let rows = answer["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{{rows: [...]}}: {answer}"));
    let mut expected: Vec<(String, String)> = Vec::new();
    for id in ["XC-B03", "XC-B02", B09] {
        let name = room
            .text(&format!("SELECT name FROM branches WHERE id = '{id}'"))
            .await
            .expect("a name");
        expected.push((name, id.to_owned()));
    }
    // XC-MAIN-like seed branches of this organisation are listed too; compare against the table itself
    let seeded: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, id FROM branches WHERE organization_id = $1::uuid AND id NOT IN ($2, 'XC-B03', 'XC-B02', $3)",
    )
    .bind(ORG)
    .bind(B01)
    .bind(B09)
    .fetch_all(&room.pool)
    .await
    .expect("the other branches of this organisation");
    expected.extend(seeded);
    expected.sort();
    let listed: Vec<(String, String)> = rows
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap_or("").to_owned(),
                r["branchId"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    assert_eq!(listed, expected, "by name, then id: {answer}");
    let ids: Vec<&str> = listed.iter().map(|(_, id)| id.as_str()).collect();
    assert!(!ids.contains(&B01), "not the session branch: {answer}");
    assert!(
        !ids.contains(&"XC-Z99"),
        "not another organisation's branch: {answer}"
    );
    for r in rows {
        assert_eq!(
            r.as_object().expect("a row").len(),
            2,
            "branchId and name only: {r}"
        );
    }

    let outsider = STAFF_ROLES
        .into_iter()
        .find(|r| !holds_any(*r, &[Permission::CashManage]))
        .expect("a role without CashManage");
    let who = room.person(outsider, Some(B01)).await;
    let a = view(&room, &who, "transfer_branches", "").await;
    assert!(a.status.is_client_error(), "{outsider:?}: {}", a.text);
    assert_eq!(code(&a), "roleNotAllowedHere", "{outsider:?}: {}", a.text);
    room.finish().await;
}

static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// LG-4 (#929, lane controller's 1:1 ruling): moves `currency amount` from `from` into `to` through the ledger
/// (`OPENING`), the store the cutover reads; before it, this file seeded `ops_state.safe` / `shift_balances`.
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
