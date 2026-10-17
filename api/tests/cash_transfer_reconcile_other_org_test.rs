//! Up-front tests (ticket author claude/studio-claude-3, on the xchange BE lane controller claude/1656900d's
//! order of 2026-10-16): `transferReconcile` (`POST /api/cash-transfers/{id}/reconcile`) must refuse a transfer
//! of another organisation. The implementer must not edit, delete or weaken these tests.
//!
//! Rows:
//! - X1. A FINANCE user of organisation B (signed in to B's branch) reconciles organisation A's RECEIVED transfer
//!   → 404 `notFound`; the transfer stays RECEIVED with no `reconciled_by`; no posting is written for it.
//! - X2 (keep). A FINANCE user of organisation A reconciles the same kind of transfer → 200, RECONCILED.
//!
//! The harness is a copy of `v3_cash_commands_test.rs`'s (that file is not edited), with `user_of` reading an
//! optional fourth `|org` part of the spec so a caller of another organisation can be signed in.

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
const ORG: &str = "00000000-0000-0000-0000-000000000001";
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
                "INSERT INTO ops_state (branch_id, phase, safe) \
                 VALUES ($1, 'OPEN', jsonb_build_object('USD', 50000::bigint))",
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
    let mut parts = spec.splitn(4, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let org = parts.next().unwrap_or(ORG);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        org,
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

enum Key {
    Fresh,
}

/// One request through the whole route table, as `who` (nobody when `None`).
async fn call(
    room: &Room,
    who: Option<&Who>,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: Key,
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

fn code(a: &Answer) -> String {
    a.body["error"]["code"].as_str().unwrap_or("").to_owned()
}

fn ok(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body.clone()
}

/// The id a command answered.
fn id_of(a: &Answer, what: &str) -> String {
    ok(a, what)["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{what} answers its record's id: {}", a.text))
        .to_owned()
}

// ------------------------------------------------------------ routes and refusals (G2, G2a)

// ------------------------------------------------------------ rows

/// Organisation B: an organisation the transfer has nothing to do with.
const ORG_B: &str = "00000000-0000-0000-0000-0000000000b2";
const B_BRANCH: &str = "ORGB-B01";

/// A BRANCH transfer XC-B01 → XC-B09 of organisation A, walked over HTTP to RECEIVED with one of two notes
/// short (so reconcile has an in-transit difference to post). Returns its id.
async fn received_transfer(room: &Room) -> String {
    let creator = room.person(Role::BranchManager, Some(B01)).await;
    let approver = room.person(Role::BranchManager, Some(B01)).await;
    let receiver = room.person(Role::BranchManager, Some(B09)).await;
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
        room,
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
    post(
        room,
        &creator,
        &format!("/api/cash-transfers/{id}/submit"),
        json!({ "branch": B01 }),
    )
    .await;
    let approver_user = user_of(&approver.spec).expect("the approver");
    TransferFlow::pg(room.pool.clone())
        .decide(&approver_user, &id, Outcome::Approved, "ok", Utc::now())
        .await
        .expect("a branch manager of the source approves within the limit");
    let a = post(
        room,
        &creator,
        &format!("/api/cash-transfers/{id}/dispatch"),
        json!({ "branch": B01 }),
    )
    .await;
    assert_eq!(a.body["status"], "DISPATCHED", "{}", a.text);
    let a = post(
        room,
        &receiver,
        &format!("/api/cash-transfers/{id}/receive"),
        json!({
            "branch": B09,
            "lines": [{ "currency": "USD", "denominationId": usd50, "qtyReceived": "1" }]
        }),
    )
    .await;
    assert_eq!(a.body["status"], "RECEIVED", "{}", a.text);
    id
}

/// A FINANCE user of organisation B, scoped to B's own branch, signed in as such.
async fn finance_of_org_b(room: &Room) -> Who {
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1::uuid, 'ORGB', 'Asia/Bangkok', now())",
    )
    .bind(ORG_B)
    .execute(&room.pool)
    .await
    .expect("organisation B");
    sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, 'B01', $2::uuid)")
        .bind(B_BRANCH)
        .bind(ORG_B)
        .execute(&room.pool)
        .await
        .expect("organisation B's branch");
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG_B)
    .bind(unique("orgb-finance"))
    .fetch_one(&room.pool)
    .await
    .expect("organisation B's user");
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
         VALUES ($1::uuid, 'FINANCE', $2::uuid, $3)",
    )
    .bind(&id)
    .bind(ORG_B)
    .bind(B_BRANCH)
    .execute(&room.pool)
    .await
    .expect("organisation B's FINANCE role");
    Who {
        spec: format!("{id}|FINANCE|{B_BRANCH}|{ORG_B}"),
        id,
    }
}

#[actix_web::test]
async fn x1_finance_of_another_organisation_cannot_reconcile_a_transfer() {
    let room = Room::start().await;
    let id = received_transfer(&room).await;
    let postings_before = room
        .text(&format!(
            "SELECT count(*)::text FROM ledger_postings WHERE source_id = '{id}'"
        ))
        .await;
    let other = finance_of_org_b(&room).await;

    let a = post(
        &room,
        &other,
        &format!("/api/cash-transfers/{id}/reconcile"),
        json!({ "branch": B_BRANCH, "note": "not mine" }),
    )
    .await;

    assert_eq!(
        a.status,
        StatusCode::NOT_FOUND,
        "X1: another organisation's transfer must answer 404 as if it did not exist: {}",
        a.text
    );
    assert_eq!(code(&a), "notFound", "X1: {}", a.text);
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM cash_transfers WHERE id = '{id}'::uuid"
        ))
        .await
        .as_deref(),
        Some("RECEIVED"),
        "X1: the transfer must stay RECEIVED"
    );
    assert_eq!(
        room.text(&format!(
            "SELECT reconciled_by::text FROM cash_transfers WHERE id = '{id}'::uuid"
        ))
        .await,
        None,
        "X1: nobody may be recorded as its reconciler"
    );
    assert_eq!(
        room.text(&format!(
            "SELECT count(*)::text FROM ledger_postings WHERE source_id = '{id}'"
        ))
        .await,
        postings_before,
        "X1: no posting may be written for the refused reconcile"
    );
    room.finish().await;
}

#[actix_web::test]
async fn x2_finance_of_the_same_organisation_still_reconciles() {
    let room = Room::start().await;
    let id = received_transfer(&room).await;
    let finance = room.person(Role::Finance, Some(B01)).await;

    let a = post(
        &room,
        &finance,
        &format!("/api/cash-transfers/{id}/reconcile"),
        json!({ "branch": B01, "note": "checked" }),
    )
    .await;

    assert_eq!(a.body["status"], "RECONCILED", "X2: {}", a.text);
    assert_eq!(
        room.text(&format!(
            "SELECT status FROM cash_transfers WHERE id = '{id}'::uuid"
        ))
        .await
        .as_deref(),
        Some("RECONCILED")
    );
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
