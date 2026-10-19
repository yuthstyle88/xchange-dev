//! Ticket #672 (TX-4) — the two settings commands over HTTP: `PUT /api/exchange-settings`
//! (`SystemSettings`) and `PUT /api/branches/{id}/exchange-limit` (`BranchesManage`). Each answers
//! the new value, refuses a caller without the permission, needs an `Idempotency-Key`, and writes one
//! `AuditLog` row whose before and after are the stored values.
//! Written before the implementation by the up-front test writer (claude/14cc24bd), on the xchange
//! BE lane controller claude/ad05bf8d's ruling on claude/5d45b017's review (#672, 2026-10-11). Do
//! not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §4.2 (the two routes, their permissions,
//! "the new settings" / "the new limit"), §3.6 (`exchange_settings.quote_ttl_seconds` 30–3600,
//! `branch_exchange_limits.approval_limit_thb` NULL or > 0), §5.2. Ticket #672 §8 ("Each writes an
//! `AuditLog` row through AP-1, with before and after"). The repository half is pinned in
//! `tx4_repositories_test.rs`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips. Each test works in a schema of its own (`tx4_common::Desk`).
//!
//! Signatures this test assumes ("choice" marks a form the design leaves open; the lane controller
//! rules on it on #672):
//! - `[required]` two `CommandAction`s in `http/v3/commands/exchange_settings.rs`, registered
//!   through the commands table: `PUT /api/exchange-settings` and `PUT
//!   /api/branches/{id}/exchange-limit`. Their policy rows (`xc-auth/.../areas/exchange_settings.rs`)
//!   need `Permission::SystemSettings` and `Permission::BranchesManage` (§4.2).
//! - choice: the settings body and answer are `{ "quoteTtlSeconds": <int> }`; outside 30–3600 is
//!   `400 validationFailed`.
//! - choice: the limit body is `{ "approvalLimitThb": "<2-place string>" | null }`; the answer is
//!   `{ "branchId", "approvalLimitThb" }`; `null` clears the limit; zero or less is
//!   `400 validationFailed` (the column's `CHECK (> 0)`).
//! - `[required]` (lane controller, #672 issuecomment-6104304961): the keys below; `null` clears the
//!   limit and is stored as NULL (T2); the audit row is AP-1's existing `AuditLog` shape.
//! - the audit row is one new `audit_events` row (AP-1's `AuditLog::append`) whose `before`
//!   and `after` are `{ "quoteTtlSeconds": n }` / `{ "approvalLimitThb": "…" | null }`, the values
//!   stored before and after the call. With no settings row, `before` is the effective 300 (§4.3
//!   step 5).
//!
//! - A caller without the permission is refused by the policy seam, with a policy code
//!   (`roleNotAllowed` / `roleNotAllowedHere`) and the status that refusal carries
//!   (`V3Error::from(PolicyError)`); nothing is written and no audit row appears.
//!
//! Callers: `owner` (the v2 owner, mapped to EXECUTIVE + ADMIN: both permissions) and `teller` (a
//! FRONT of MAIN, mapped to TELLER: neither). The premise is checked against `RoleGrants`, not
//! assumed.

mod common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use xc_auth::domain::access::{Permission, Role as AccessRole, RoleGrants};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const SETTINGS: &str = "/api/exchange-settings";
const LIMIT: &str = "/api/branches/MAIN/exchange-limit";

// ------------------------------------------------------------ callers

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    match spec {
        "owner" => Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        },
        "teller" => Sessions {
            owner: None,
            staff: Actor::staff(TELLER, Role::Front, BranchId::new("MAIN")),
        },
        _ => Sessions {
            owner: None,
            staff: None,
        },
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
    }
}

/// A policy refusal: a role code and a client-error status (`PolicyError::status`).
fn policy_refused(a: &Answer, what: &str) {
    assert!(
        a.status.is_client_error() && matches!(a.code(), "roleNotAllowed" | "roleNotAllowedHere"),
        "{what}: a policy refusal, got {} {}",
        a.status,
        a.text
    );
}

/// The premise of the refusal tests: the teller's role lacks `p` and the owner's roles hold it.
fn premise(p: Permission) {
    // Follows §11.2 seed: TELLER lacks and ADMIN holds the settings permissions
    assert!(
        !RoleGrants::seed(AccessRole::Teller).contains(&p),
        "premise: TELLER lacks {p:?}"
    );
    assert!(
        RoleGrants::seed(AccessRole::Admin).contains(&p),
        "premise: ADMIN (the owner's) holds {p:?}"
    );
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

async fn put(desk: &Desk, uri: &str, who: &str, body: &Value, key: Option<&str>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(Method::PUT)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .set_payload(body.to_string());
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    if let Some(key) = key {
        req = req.insert_header(("idempotency-key", key));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn call(desk: &Desk, uri: &str, who: &str, body: &Value) -> Answer {
    put(desk, uri, who, body, Some(&unique("key"))).await
}

// ------------------------------------------------------------ stored values

/// The organisation's stored TTL; 300 when it has no row (§4.3 step 5).
async fn ttl(desk: &Desk) -> i64 {
    desk.db
        .text(&format!(
            "SELECT max(quote_ttl_seconds)::text FROM exchange_settings WHERE organization_id = '{ORG_1}'"
        ))
        .await
        .map(|t| t.parse().unwrap())
        .unwrap_or(300)
}

/// MAIN's stored limit as a 2-place string, `None` when unset.
async fn limit(desk: &Desk) -> Option<String> {
    desk.db
        .text(&format!(
            "SELECT max(approval_limit_thb)::text FROM branch_exchange_limits \
             WHERE organization_id = '{ORG_1}' AND branch_id = 'MAIN' AND approval_limit_thb IS NOT NULL"
        ))
        .await
}

async fn audit_rows(desk: &Desk) -> i64 {
    desk.db.count("SELECT count(*) FROM audit_events").await
}

/// The newest audit row's (before, after).
async fn last_audit(desk: &Desk) -> (Value, Value) {
    let row: (Option<Value>, Option<Value>) = sqlx::query_as(
        "SELECT before_redacted, after_redacted FROM audit_events ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&desk.db.pool)
    .await
    .expect("an audit row");
    (row.0.unwrap_or(Value::Null), row.1.unwrap_or(Value::Null))
}

// ------------------------------------------------------------ PUT /api/exchange-settings

#[actix_web::test]
async fn s1_settings_answer_the_new_ttl_store_it_and_write_one_audit_row() {
    let desk = Desk::start().await;
    let before = ttl(&desk).await;
    let audits = audit_rows(&desk).await;
    let a = call(&desk, SETTINGS, "owner", &json!({ "quoteTtlSeconds": 600 })).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["quoteTtlSeconds"], json!(600), "the answer");
    assert_eq!(ttl(&desk).await, 600, "stored");
    assert_eq!(audit_rows(&desk).await, audits + 1, "one audit row");
    let (b, af) = last_audit(&desk).await;
    assert_eq!(
        b,
        json!({ "quoteTtlSeconds": before }),
        "before = the stored value"
    );
    assert_eq!(
        af,
        json!({ "quoteTtlSeconds": 600 }),
        "after = the stored value"
    );
}

#[actix_web::test]
async fn s2_settings_refuse_a_caller_without_system_settings() {
    premise(Permission::SystemSettings);
    let desk = Desk::start().await;
    let audits = audit_rows(&desk).await;
    let a = call(
        &desk,
        SETTINGS,
        "teller",
        &json!({ "quoteTtlSeconds": 600 }),
    )
    .await;
    policy_refused(&a, "a teller saving the settings");
    assert_eq!(ttl(&desk).await, 300, "unchanged");
    assert_eq!(audit_rows(&desk).await, audits, "no audit row");
}

#[actix_web::test]
async fn s3_settings_without_an_idempotency_key_are_refused() {
    let desk = Desk::start().await;
    let audits = audit_rows(&desk).await;
    let a = put(
        &desk,
        SETTINGS,
        "owner",
        &json!({ "quoteTtlSeconds": 600 }),
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "body {}", a.text);
    assert_eq!(ttl(&desk).await, 300, "unchanged");
    assert_eq!(audit_rows(&desk).await, audits, "no audit row");
}

#[actix_web::test]
async fn s4_a_ttl_outside_30_to_3600_is_a_validation_failure() {
    let desk = Desk::start().await;
    let audits = audit_rows(&desk).await;
    for seconds in [29, 3601] {
        let a = call(
            &desk,
            SETTINGS,
            "owner",
            &json!({ "quoteTtlSeconds": seconds }),
        )
        .await;
        assert_eq!(
            (a.status, a.code()),
            (StatusCode::BAD_REQUEST, "validationFailed"),
            "{seconds}: body {}",
            a.text
        );
    }
    for seconds in [30, 3600] {
        let a = call(
            &desk,
            SETTINGS,
            "owner",
            &json!({ "quoteTtlSeconds": seconds }),
        )
        .await;
        assert_eq!(a.status, StatusCode::OK, "{seconds} is allowed: {}", a.text);
        assert_eq!(ttl(&desk).await, seconds, "{seconds} stored");
    }
    assert_eq!(
        audit_rows(&desk).await,
        audits + 2,
        "only the two saves audited"
    );
}

// ------------------------------------------------------------ PUT /api/branches/{id}/exchange-limit

#[actix_web::test]
async fn l1_the_limit_answers_the_branch_and_new_limit_stores_it_and_writes_one_audit_row() {
    let desk = Desk::start().await;
    let audits = audit_rows(&desk).await;
    let a = call(
        &desk,
        LIMIT,
        "owner",
        &json!({ "approvalLimitThb": "5000.00" }),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["branchId"], json!("MAIN"), "branchId");
    assert_eq!(
        a.body["approvalLimitThb"],
        json!("5000.00"),
        "approvalLimitThb"
    );
    assert_eq!(limit(&desk).await.as_deref(), Some("5000.00"), "stored");
    assert_eq!(audit_rows(&desk).await, audits + 1, "one audit row");
    let (b, af) = last_audit(&desk).await;
    assert_eq!(b, json!({ "approvalLimitThb": null }), "before: no limit");
    assert_eq!(
        af,
        json!({ "approvalLimitThb": "5000.00" }),
        "after: the stored limit"
    );
}

#[actix_web::test]
async fn l2_null_clears_the_limit_and_is_audited_with_the_old_value() {
    let desk = Desk::start().await;
    desk.limit("MAIN", "5000").await;
    let audits = audit_rows(&desk).await;
    let a = call(&desk, LIMIT, "owner", &json!({ "approvalLimitThb": null })).await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["approvalLimitThb"], Value::Null, "cleared");
    assert_eq!(limit(&desk).await, None, "no limit stored");
    // T2 (quote-transaction.md:253): `null` is stored as NULL, "no transaction waits"; the row stays
    assert_eq!(
        desk.db
            .count(&format!(
                "SELECT count(*) FROM branch_exchange_limits \
                 WHERE organization_id = '{ORG_1}' AND branch_id = 'MAIN' \
                   AND approval_limit_thb IS NULL"
            ))
            .await,
        1,
        "MAIN's row holds a NULL limit"
    );
    assert_eq!(audit_rows(&desk).await, audits + 1, "one audit row");
    let (b, af) = last_audit(&desk).await;
    assert_eq!(
        b,
        json!({ "approvalLimitThb": "5000.00" }),
        "before: the old limit"
    );
    assert_eq!(af, json!({ "approvalLimitThb": null }), "after: none");
}

#[actix_web::test]
async fn l3_the_limit_refuses_a_caller_without_branches_manage() {
    premise(Permission::BranchesManage);
    let desk = Desk::start().await;
    let audits = audit_rows(&desk).await;
    let a = call(
        &desk,
        LIMIT,
        "teller",
        &json!({ "approvalLimitThb": "5000.00" }),
    )
    .await;
    policy_refused(&a, "a teller saving the limit");
    assert_eq!(limit(&desk).await, None, "nothing stored");
    assert_eq!(audit_rows(&desk).await, audits, "no audit row");
}

#[actix_web::test]
async fn l4_the_limit_without_an_idempotency_key_is_refused() {
    let desk = Desk::start().await;
    let audits = audit_rows(&desk).await;
    let a = put(
        &desk,
        LIMIT,
        "owner",
        &json!({ "approvalLimitThb": "5000.00" }),
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "body {}", a.text);
    assert_eq!(limit(&desk).await, None, "nothing stored");
    assert_eq!(audit_rows(&desk).await, audits, "no audit row");
}

#[actix_web::test]
async fn l5_a_zero_limit_is_a_validation_failure() {
    let desk = Desk::start().await;
    let audits = audit_rows(&desk).await;
    let a = call(
        &desk,
        LIMIT,
        "owner",
        &json!({ "approvalLimitThb": "0.00" }),
    )
    .await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::BAD_REQUEST, "validationFailed"),
        "body {}",
        a.text
    );
    assert_eq!(limit(&desk).await, None, "nothing stored");
    assert_eq!(audit_rows(&desk).await, audits, "no audit row");
}
