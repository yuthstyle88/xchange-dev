//! A malformed stored cash-transfer policy (`cash_transfer_policy_versions.limits`, jsonb) must never be read as
//! "no limits", and the save must never write a value its readers cannot read.
//!
//! Written before the fix by the test-writer claude/7c9f3ae3, on the BE lane controller's ruling. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! **The rulings pinned here.**
//! - (a) Every reader of the stored limits refuses a value it cannot read: `AppError::Internal`, which the seams
//!   answer `500 serverError`, logged; never an empty map. The readers are the policy view
//!   (`views/cash_transfer_policy.rs:111-112`, `serde_json::from_value(..).unwrap_or_default()`) and the reader the
//!   transfer submit enforces with (`xc-cash/src/infrastructure/transfer_repository.rs:379-421`
//!   `find_latest_policy`, which skips every entry it cannot parse and turns a non-object into an empty map).
//! - (b) `transferPolicySave` refuses a limit its readers could not read with `400 validationFailed` and a field
//!   name, and writes nothing.
//!
//! **What "malformed" means here (choice of this file):** any stored `limits` that today's design shape
//! `{CUR: str}` (design `docs/design/blueprint/cash-shift-transfer.md` §4.4 `:293`, §4.7 `:398`) does not describe,
//! or that the fixed save would refuse: not an object; a value that is not a string (number, null, bool, object); a
//! string that is not a plain non-negative amount `Money::parse` reads (`abc`, `-5.00`, ` 300.00`, more than
//! `Money::MAX_INPUT_UNITS`); a key that is not a known currency code exactly (`ZZZ`, ` USD`). `{}` is well formed
//! and means "no currency has a limit", i.e. everything is above the limit (`TransferPolicy::is_within_limit`).
//!
//! **What today's code does with a malformed value** (measured on main, not a bypass of the approval itself):
//! the submit reader drops what it cannot parse, and a currency with no limit counts as above it, so the transfer
//! goes to EXECUTIVE. A *partly* malformed value keeps the entries it could parse, so the transfer is decided on a
//! policy nobody saved; the view answers `{}` for the whole value. The tests below pin the rulings, not that.
//!
//! The submit answers through `POST /api/cash-transfers/{id}/submit`; the approval decision is made in-process with
//! `TransferFlow::decide`, as `v3_cash_commands_test.rs` does (PR 570's route is not this file's). Needs
//! `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips. Each test works in a schema of
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
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
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
/// The seeded organisation (`20261011100000_organizations.sql`).
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const POLICY_ROUTE: &str = "/api/cash-transfer-policy";
const POLICY_VIEW: &str = "cash_transfer_policy";

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

/// Stored `limits` values no reader may read as a policy (ruling (a)), each with why.
fn malformed_stored() -> Vec<(&'static str, Value)> {
    vec![
        ("an array, not an object", json!([])),
        ("a string, not an object", json!("300.00")),
        ("a JSON null value for the whole policy", Value::Null),
        (
            "a number where the amount string goes",
            json!({ "USD": 300 }),
        ),
        ("null where the amount string goes", json!({ "USD": null })),
        (
            "an object where the amount string goes",
            json!({ "USD": { "amount": "300.00" } }),
        ),
        ("a string that is not an amount", json!({ "USD": "abc" })),
        ("a negative amount", json!({ "USD": "-5.00" })),
        ("an amount with padding", json!({ "USD": " 300.00" })),
        (
            "an amount past Money::MAX_INPUT_UNITS",
            json!({ "USD": "1000000000.00" }),
        ),
        ("an unknown currency code", json!({ "ZZZ": "300.00" })),
        ("a currency code with padding", json!({ " USD": "300.00" })),
        (
            "one good entry beside a malformed one",
            json!({ "USD": "300.00", "EUR": true }),
        ),
    ]
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
        let schema = unique("xclimitsbad");
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
            fund(&pool, branch, "USD", "500.00").await;
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

    /// Replaces every policy version with one row whose `limits` is `limits`, written by `by`.
    async fn store_policy(&self, limits: &Value, by: &Who) {
        sqlx::query("DELETE FROM cash_transfer_policy_versions")
            .execute(&self.pool)
            .await
            .expect("clear the policy versions");
        sqlx::query(
            "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
             VALUES ($1, $2::uuid, $3, $4::uuid)",
        )
        .bind(unique("v"))
        .bind(ORG)
        .bind(limits)
        .bind(&by.id)
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("store the policy {limits}: {e}"));
    }

    async fn policy_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM cash_transfer_policy_versions")
            .fetch_one(&self.pool)
            .await
            .expect("count the policy versions")
    }

    async fn usd50(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50::numeric",
        )
        .fetch_one(&self.pool)
        .await
        .expect("the USD 50 face value")
    }

    /// `(status, approval_request_id is set, approval_requests rows for it)` of a transfer.
    async fn transfer_state(&self, id: &str) -> (String, bool, i64) {
        let (status, has_request): (String, bool) = sqlx::query_as(
            "SELECT status, approval_request_id IS NOT NULL FROM cash_transfers WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("read the transfer");
        let requests: i64 =
            sqlx::query_scalar("SELECT count(*) FROM approval_requests WHERE object_id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .expect("count the transfer's approval requests");
        (status, has_request, requests)
    }

    /// The `within_limit` the submit recorded on the transfer's approval request.
    async fn within_limit(&self, id: &str) -> Option<bool> {
        sqlx::query_scalar(
            "SELECT (proposed_payload->>'within_limit')::boolean FROM approval_requests \
             WHERE object_id = $1",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("the transfer's approval request")
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

/// Puts `amount` of `currency` into the branch's safe through the ledger (LG-4), as the cash tests do.
async fn fund(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
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
                        "limits-fund:{currency}:{}",
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

/// One request through the whole route table as `who`; a body goes with a fresh `Idempotency-Key`.
async fn call(room: &Room, who: &Who, method: Method, uri: &str, body: Option<Value>) -> Answer {
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
        .insert_header((AS, who.spec.clone()));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
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

fn code(a: &Answer) -> String {
    a.body["error"]["code"].as_str().unwrap_or("").to_owned()
}

fn field(a: &Answer) -> String {
    a.body["error"]["field"].as_str().unwrap_or("").to_owned()
}

/// A DRAFT transfer of USD 100.00 (two USD 50 notes) from B01 to B09, made by `creator`.
async fn draft_transfer(room: &Room, creator: &Who) -> String {
    let usd50 = room.usd50().await;
    let a = call(
        room,
        creator,
        Method::POST,
        "/api/cash-transfers",
        Some(json!({
            "branch": B01,
            "kind": "BRANCH",
            "toBranchId": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qty": "2" }],
            "note": "limits test"
        })),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "transferCreate: {}", a.text);
    a.body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("transferCreate answers an id: {}", a.text))
        .to_owned()
}

async fn submit(room: &Room, creator: &Who, id: &str) -> Answer {
    call(
        room,
        creator,
        Method::POST,
        &format!("/api/cash-transfers/{id}/submit"),
        Some(json!({ "branch": B01 })),
    )
    .await
}

async fn read_view(room: &Room, who: &Who) -> Answer {
    call(
        room,
        who,
        Method::GET,
        &format!("/api/v3/views/{POLICY_VIEW}?branch={B01}"),
        None,
    )
    .await
}

async fn save(room: &Room, who: &Who, limits: Value) -> Answer {
    call(
        room,
        who,
        Method::PUT,
        POLICY_ROUTE,
        Some(json!({ "branch": B01, "limits": limits })),
    )
    .await
}

async fn decide(room: &Room, who: &Who, id: &str) -> Result<(), String> {
    let user = user_of(&who.spec).expect("the approver");
    TransferFlow::pg(room.pool.clone())
        .decide(&user, id, Outcome::Approved, "ok", Utc::now())
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

// ------------------------------------------------------------ ruling (a): the enforcing reader

/// Ruling (a) on the submit path: with a malformed stored policy the submit answers `500 serverError` and writes
/// nothing — the transfer stays DRAFT with no approval request. Today the reader drops what it cannot parse and the
/// submit answers 200 with an approval request decided on a policy nobody saved.
#[actix_web::test]
async fn a_malformed_stored_policy_refuses_transfer_submit_with_server_error_and_writes_nothing() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    let mut wrong = Vec::new();
    for (why, limits) in malformed_stored() {
        room.store_policy(&limits, &executive).await;
        let id = draft_transfer(&room, &creator).await;
        let a = submit(&room, &creator, &id).await;
        let state = room.transfer_state(&id).await;
        if a.status != StatusCode::INTERNAL_SERVER_ERROR || code(&a) != "serverError" {
            wrong.push(format!(
                "{why} {limits}: submit answered {} {} (state {state:?})",
                a.status, a.text
            ));
            continue;
        }
        if state != ("DRAFT".to_owned(), false, 0) {
            wrong.push(format!(
                "{why} {limits}: the refused submit wrote something: (status, has request, requests) = {state:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "a malformed policy must refuse the submit (500 serverError, nothing written):\n{}",
        wrong.join("\n")
    );
    room.finish().await;
}

/// The partly malformed case on its own, because it is where today's reader decides on a policy nobody saved:
/// `{"USD": "300.00", "EUR": true}` keeps USD 300.00, so a USD 100.00 transfer is recorded within the limit and a
/// branch manager of the source may approve it. Ruling (a): the submit is refused instead, so nobody approves.
#[actix_web::test]
async fn a_partly_malformed_policy_is_not_honoured_entry_by_entry() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    room.store_policy(&json!({ "USD": "300.00", "EUR": true }), &executive)
        .await;
    let id = draft_transfer(&room, &creator).await;
    let a = submit(&room, &creator, &id).await;
    let decided = decide(&room, &approver, &id).await;
    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::INTERNAL_SERVER_ERROR, "serverError"),
        "the submit is refused: {} (then a branch manager's decision: {decided:?})",
        a.text
    );
    assert!(
        decided.is_err(),
        "no approval request exists, so a branch manager cannot approve"
    );
    assert_eq!(
        room.transfer_state(&id).await,
        ("DRAFT".to_owned(), false, 0)
    );
    room.finish().await;
}

// ------------------------------------------------------------ keep-pins: well-formed policies still enforce

/// Keep-pin: a well-formed policy `{"USD": "300.00"}` records a USD 100.00 transfer within the limit, and a branch
/// manager of the source approves it.
#[actix_web::test]
async fn a_well_formed_policy_still_lets_a_branch_manager_approve_within_the_limit() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    room.store_policy(&json!({ "USD": "300.00" }), &executive)
        .await;
    let id = draft_transfer(&room, &creator).await;
    let a = submit(&room, &creator, &id).await;
    assert_eq!(a.status, StatusCode::OK, "the submit: {}", a.text);
    assert_eq!(room.within_limit(&id).await, Some(true));
    decide(&room, &approver, &id)
        .await
        .expect("a branch manager of the source approves within the limit");
    assert_eq!(room.transfer_state(&id).await.0, "APPROVED");
    room.finish().await;
}

/// Keep-pin: a well-formed policy `{"USD": "50.00"}` records a USD 100.00 transfer above the limit; a branch
/// manager is refused and EXECUTIVE approves.
#[actix_web::test]
async fn a_well_formed_policy_still_sends_a_transfer_above_the_limit_to_the_executive() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    room.store_policy(&json!({ "USD": "50.00" }), &executive)
        .await;
    let id = draft_transfer(&room, &creator).await;
    let a = submit(&room, &creator, &id).await;
    assert_eq!(a.status, StatusCode::OK, "the submit: {}", a.text);
    assert_eq!(room.within_limit(&id).await, Some(false));
    assert!(
        decide(&room, &approver, &id).await.is_err(),
        "a branch manager may not approve above the limit"
    );
    decide(&room, &executive, &id)
        .await
        .expect("EXECUTIVE approves above the limit");
    assert_eq!(room.transfer_state(&id).await.0, "APPROVED");
    room.finish().await;
}

/// Keep-pin: `{}` is well formed (no currency has a limit), so the submit answers 200 and records the transfer
/// above the limit; the view answers it as `{}`. The fix must not refuse an empty policy.
#[actix_web::test]
async fn an_empty_policy_object_is_well_formed_and_means_every_transfer_is_above_the_limit() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    room.store_policy(&json!({}), &executive).await;
    let id = draft_transfer(&room, &creator).await;
    let a = submit(&room, &creator, &id).await;
    assert_eq!(a.status, StatusCode::OK, "the submit: {}", a.text);
    assert_eq!(room.within_limit(&id).await, Some(false));
    let v = read_view(&room, &executive).await;
    assert_eq!(v.status, StatusCode::OK, "the view: {}", v.text);
    assert_eq!(v.body["data"]["limits"], json!({}), "{}", v.text);
    room.finish().await;
}

// ------------------------------------------------------------ ruling (a): the view

/// Ruling (a) on the view (`views/cash_transfer_policy.rs:111-112`): a malformed stored policy answers
/// `500 serverError`, never `200` with `limits: {}`.
#[actix_web::test]
async fn the_policy_view_answers_server_error_for_a_malformed_stored_policy() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    let mut wrong = Vec::new();
    for (why, limits) in malformed_stored() {
        room.store_policy(&limits, &executive).await;
        let a = read_view(&room, &executive).await;
        if a.status != StatusCode::INTERNAL_SERVER_ERROR || code(&a) != "serverError" {
            wrong.push(format!(
                "{why} {limits}: the view answered {} {}",
                a.status, a.text
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "a malformed policy must make the view answer 500 serverError:\n{}",
        wrong.join("\n")
    );
    room.finish().await;
}

/// Keep-pin: a well-formed stored policy is answered as stored.
#[actix_web::test]
async fn the_policy_view_still_answers_a_well_formed_policy() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    let limits = json!({ "USD": "300.00", "THB": "1000000.00" });
    room.store_policy(&limits, &executive).await;
    let a = read_view(&room, &executive).await;
    assert_eq!(a.status, StatusCode::OK, "the view: {}", a.text);
    assert_eq!(a.body["data"]["limits"], limits, "{}", a.text);
    room.finish().await;
}

// ------------------------------------------------------------ ruling (b): the save

/// Ruling (b): `transferPolicySave` refuses a limit its readers could not read with `400 validationFailed` and a
/// field name, and writes nothing. The field names are the ones the save already uses
/// (`commands/cash.rs:319-338`): `currency` for a key, `limits` for an amount. Today each of these passes the
/// save's checks (it trims before checking, then stores the untrimmed text, and checks with `Decimal::from_str`
/// instead of `Money::parse`) and writes a row the submit reader drops.
#[actix_web::test]
async fn the_save_refuses_a_limit_its_readers_cannot_read_with_validation_failed_and_a_field() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "an amount with padding",
            json!({ "USD": " 300.00" }),
            "limits",
        ),
        (
            "an amount with a plus sign",
            json!({ "USD": "+300.00" }),
            "limits",
        ),
        (
            "an amount past Money::MAX_INPUT_UNITS",
            json!({ "USD": "1000000000.00" }),
            "limits",
        ),
        (
            "a currency code with padding",
            json!({ " USD": "300.00" }),
            "currency",
        ),
        (
            "two keys naming one currency once trimmed",
            json!({ "USD": "300.00", "USD ": "9000.00" }),
            "currency",
        ),
    ];
    let mut wrong = Vec::new();
    for (why, limits, want_field) in cases {
        let a = save(&room, &executive, limits.clone()).await;
        if a.status != StatusCode::BAD_REQUEST
            || code(&a) != "validationFailed"
            || field(&a) != want_field
        {
            wrong.push(format!(
                "{why} {limits}: the save answered {} {} (want 400 validationFailed field {want_field})",
                a.status, a.text
            ));
        }
    }
    let rows = room.policy_rows().await;
    assert!(
        wrong.is_empty() && rows == 0,
        "the save must refuse what its readers cannot read ({rows} rows written):\n{}",
        wrong.join("\n")
    );
    room.finish().await;
}

/// Keep-pin: what the save refuses today it still refuses, with the same code and field: a non-numeric amount, a
/// negative amount (`limits`), an unknown currency (`currency`).
#[actix_web::test]
async fn the_save_still_refuses_what_it_refuses_today() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "a string that is not an amount",
            json!({ "USD": "abc" }),
            "limits",
        ),
        ("a negative amount", json!({ "USD": "-5.00" }), "limits"),
        (
            "an unknown currency code",
            json!({ "ZZZ": "300.00" }),
            "currency",
        ),
    ];
    for (why, limits, want_field) in cases {
        let a = save(&room, &executive, limits.clone()).await;
        assert_eq!(
            (a.status, code(&a), field(&a)),
            (
                StatusCode::BAD_REQUEST,
                "validationFailed".to_owned(),
                want_field.to_owned()
            ),
            "{why} {limits}: {}",
            a.text
        );
    }
    assert_eq!(room.policy_rows().await, 0);
    room.finish().await;
}

/// Keep-pin: a JSON value that is not the Input type (`{CUR: str}`) is refused by the seam before the save runs
/// (`http/v3/command.rs`, the G1 ruling on #624: `invalidRequest`), and writes nothing. Only the status and the
/// row count are pinned here; the code is the seam's.
#[actix_web::test]
async fn a_limits_value_that_is_not_the_input_type_is_refused_400_and_writes_nothing() {
    let room = Room::start().await;
    let executive = room.person(Role::Executive, None).await;
    for limits in [
        json!({ "USD": 300 }),
        json!({ "USD": null }),
        json!([]),
        json!("300.00"),
    ] {
        let a = save(&room, &executive, limits.clone()).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST, "{limits}: {}", a.text);
    }
    assert_eq!(room.policy_rows().await, 0);
    room.finish().await;
}

/// Keep-pin, end to end: a well-formed save answers 200, the view answers what was saved, and the submit enforces
/// it (USD 100.00 within `"300"`; `"0.00"` for EUR is a limit too). What the save writes, the fixed readers read.
#[actix_web::test]
async fn a_well_formed_save_is_answered_200_read_back_and_enforced() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    let limits = json!({ "USD": "300", "EUR": "0.00", "THB": "1000000.00" });
    let a = save(&room, &executive, limits.clone()).await;
    assert_eq!(a.status, StatusCode::OK, "the save: {}", a.text);
    assert_eq!(a.body["limits"], limits, "{}", a.text);
    let v = read_view(&room, &executive).await;
    assert_eq!(v.status, StatusCode::OK, "the view: {}", v.text);
    assert_eq!(v.body["data"]["limits"], limits, "{}", v.text);
    let id = draft_transfer(&room, &creator).await;
    let s = submit(&room, &creator, &id).await;
    assert_eq!(s.status, StatusCode::OK, "the submit: {}", s.text);
    assert_eq!(room.within_limit(&id).await, Some(true));
    room.finish().await;
}

// ------------------------------------------------------------ the dropped-entry reader (find_latest_policy)

/// Ruling (a) on `find_latest_policy`'s per-entry drop (`transfer_repository.rs:400-418` on `81e22fe4`): the
/// limit of the currency the transfer moves is unparseable (`{"USD": "abc", "EUR": "300.00"}`, USD transfer).
/// The submit answers `500 serverError`, no approval request is written, so nobody approves it and it cannot be
/// dispatched. Today the reader drops USD and the submit answers 200 (the transfer then needs EXECUTIVE).
#[actix_web::test]
async fn an_unparseable_limit_for_the_moved_currency_refuses_the_submit_and_nothing_is_approved_or_dispatched()
 {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    room.store_policy(&json!({ "USD": "abc", "EUR": "300.00" }), &executive)
        .await;
    let id = draft_transfer(&room, &creator).await;
    let a = submit(&room, &creator, &id).await;
    let by_manager = decide(&room, &approver, &id).await;
    let by_executive = decide(&room, &executive, &id).await;
    let dispatched = call(
        &room,
        &creator,
        Method::POST,
        &format!("/api/cash-transfers/{id}/dispatch"),
        Some(json!({ "branch": B01 })),
    )
    .await;
    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::INTERNAL_SERVER_ERROR, "serverError"),
        "the submit is refused: {} (then manager {by_manager:?}, executive {by_executive:?}, dispatch {} {})",
        a.text,
        dispatched.status,
        dispatched.text
    );
    assert!(
        by_manager.is_err() && by_executive.is_err(),
        "nothing to approve"
    );
    assert_ne!(dispatched.status, StatusCode::OK, "{}", dispatched.text);
    assert_eq!(
        room.transfer_state(&id).await,
        ("DRAFT".to_owned(), false, 0)
    );
    room.finish().await;
}

/// Keep-pin: a well-formed policy over several currencies still enforces each currency's own limit — USD 100.00 is
/// above `"USD": "50.00"` even though EUR's limit is large, and within `"USD": "300.00"` even though EUR's is 0.
#[actix_web::test]
async fn a_well_formed_policy_over_several_currencies_still_enforces_each_currency() {
    let room = Room::start().await;
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let executive = room.person(Role::Executive, None).await;
    for (limits, within) in [
        (json!({ "USD": "50.00", "EUR": "100000.00" }), false),
        (json!({ "USD": "300.00", "EUR": "0.00" }), true),
    ] {
        room.store_policy(&limits, &executive).await;
        let id = draft_transfer(&room, &creator).await;
        let a = submit(&room, &creator, &id).await;
        assert_eq!(a.status, StatusCode::OK, "{limits}: {}", a.text);
        assert_eq!(room.within_limit(&id).await, Some(within), "{limits}");
    }
    room.finish().await;
}
