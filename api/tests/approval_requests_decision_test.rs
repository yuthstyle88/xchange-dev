//! The A06 approval queue shows each request's decision: who decided, the outcome, the reason and when.
//! Written before the implementation (claude/c4d72c, approved by the BE lane controller claude/7ca80c,
//! 2026-10-15). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design `docs/design/blueprint/rates.md:452`: `GET /api/v3/views/approval_requests` answers
//! "requests with proposer, values before and after, reason"; `:444` A06 decides "APPROVED or REJECTED
//! with a reason". On main the item DTO (`presentation/dto/v3/approval_requests.rs`) has no decision
//! field and the view never reads `approval_decisions` (migration `20261010110000_approvals.sql:17-25`).
//!
//! The shape pinned here ([required], approved by the controller): every item carries
//! `decision: {approverId, outcome, reason, at} | null`. `null` while the request has no decision row;
//! `at` is RFC 3339 like `requestedAt`. The DTO is `ApprovalDecisionDto`, exported to
//! `approval_requests.ts`, and the committed generated file is what the DTOs generate.
//!
//! Needs `TEST_DATABASE_URL`; the database tests do not skip. Sessions come from the `x-test-as`
//! header, as in `approvals_read_per_type_test.rs`.

mod common;
mod quote_common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::TestDb;
use serde_json::{Value, json};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const VIEW: &str = "approval_requests";

/// Every request is read as the owner, who decides rate publications.
struct OwnerCallers;

impl SessionReader for OwnerCallers {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move { test_callers::callers(Some(test_callers::owner("owner-1")), None) })
    }
}

/// The listed requests, in the view's order; panics unless the view answers 200.
async fn listed(db: &TestDb) -> Vec<Value> {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(OwnerCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri("/api/v3/views/approval_requests")
        .insert_header((header::HOST, "xc.test"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "{text}");
    let body: Value = serde_json::from_slice(&bytes).expect("JSON");
    body.pointer("/data/requests")
        .or_else(|| body.pointer("/requests"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no requests list in {text}"))
        .clone()
}

/// A RATE_PUBLICATION request at MAIN named `obj`, in `status`, requested at `at`.
async fn request(db: &TestDb, obj: &str, status: &str, at: &str) -> String {
    db
        .text(&format!(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
                policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLICATION', 'MAIN', '{obj}', 'h', '1', '{{}}', \
                'proposer-1', '{at}', '{status}', 'k-{obj}') RETURNING id::text"
        ))
        .await
        .expect("a request id")
}

/// The decision row of `request`.
async fn decision(
    db: &TestDb,
    request: &str,
    approver: &str,
    outcome: &str,
    reason: &str,
    at: &str,
) {
    db.exec(&format!(
        "INSERT INTO approval_decisions (id, request_id, approver_id, approver_role, outcome, reason, at) \
         VALUES (gen_random_uuid(), '{request}', '{approver}', 'OWNER', '{outcome}', '{reason}', '{at}')"
    ))
    .await;
}

fn item<'a>(items: &'a [Value], obj: &str) -> &'a Value {
    items
        .iter()
        .find(|r| r["objectId"] == obj)
        .unwrap_or_else(|| panic!("{obj} is listed: {items:?}"))
}

#[tokio::test]
async fn a_pending_request_answers_a_null_decision() {
    let db = TestDb::start().await;
    request(&db, "obj-pending", "PENDING", "2026-10-15T01:00:00Z").await;

    let items = listed(&db).await;
    let r = item(&items, "obj-pending");
    assert!(
        r.as_object().expect("an object").contains_key("decision"),
        "every item carries `decision`, null while undecided: {r}"
    );
    assert_eq!(r["decision"], Value::Null, "no decision row ⇒ null");
    db.finish().await;
}

#[tokio::test]
async fn an_approved_request_answers_who_decided_why_and_when() {
    let db = TestDb::start().await;
    let id = request(&db, "obj-approved", "APPROVED", "2026-10-15T01:00:00Z").await;
    decision(
        &db,
        &id,
        "owner-9",
        "APPROVED",
        "rates checked",
        "2026-10-15T02:03:04Z",
    )
    .await;

    let items = listed(&db).await;
    assert_eq!(
        item(&items, "obj-approved")["decision"],
        json!({
            "approverId": "owner-9",
            "outcome": "APPROVED",
            "reason": "rates checked",
            "at": "2026-10-15T02:03:04+00:00",
        }),
        "rates.md:452: the queue shows the decision and its reason"
    );
    db.finish().await;
}

#[tokio::test]
async fn each_request_answers_its_own_decision() {
    let db = TestDb::start().await;
    let rejected = request(&db, "obj-rejected", "REJECTED", "2026-10-15T01:00:00Z").await;
    let approved = request(&db, "obj-other", "APPROVED", "2026-10-15T01:30:00Z").await;
    request(&db, "obj-waiting", "PENDING", "2026-10-15T01:45:00Z").await;
    decision(
        &db,
        &rejected,
        "owner-1",
        "REJECTED",
        "spread too wide",
        "2026-10-15T03:00:00Z",
    )
    .await;
    decision(
        &db,
        &approved,
        "owner-2",
        "APPROVED",
        "fine",
        "2026-10-15T03:30:00Z",
    )
    .await;

    let items = listed(&db).await;
    assert_eq!(items.len(), 3, "a decision row adds no item: {items:?}");
    let r = &item(&items, "obj-rejected")["decision"];
    assert_eq!(r["approverId"], "owner-1");
    assert_eq!(r["outcome"], "REJECTED");
    assert_eq!(r["reason"], "spread too wide");
    assert_eq!(r["at"], "2026-10-15T03:00:00+00:00");
    assert_eq!(
        item(&items, "obj-other")["decision"]["approverId"],
        "owner-2"
    );
    assert_eq!(item(&items, "obj-waiting")["decision"], Value::Null);
    db.finish().await;
}

#[tokio::test]
async fn the_decision_is_read_from_the_decision_row_not_the_request_status() {
    // The two columns are made to disagree on purpose, so that only a value read from
    // `approval_decisions` passes; `decide` writes them together in practice.
    let db = TestDb::start().await;
    let id = request(&db, "obj-split", "APPROVED", "2026-10-15T01:00:00Z").await;
    decision(
        &db,
        &id,
        "owner-3",
        "REJECTED",
        "row wins",
        "2026-10-15T04:00:00Z",
    )
    .await;

    let items = listed(&db).await;
    let r = item(&items, "obj-split");
    assert_eq!(
        r["status"], "APPROVED",
        "the request's own status is unchanged"
    );
    assert_eq!(
        r["decision"]["outcome"], "REJECTED",
        "decision.outcome is the decision row's"
    );
    db.finish().await;
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        // One directory per call: the two type tests run on parallel threads of one binary, and a
        // name made of the process id alone let each one's `remove_dir_all` delete the other's.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "approval-requests-decision-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create a scratch directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exported() -> String {
    let view = VIEWS
        .iter()
        .find(|v| v.name == VIEW)
        .unwrap_or_else(|| panic!("{VIEW} is in the generated view table"));
    let scratch = Scratch::new();
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {VIEW}: {e}"));
    std::fs::read_to_string(scratch.0.join(format!("{VIEW}.ts")))
        .unwrap_or_else(|e| panic!("{VIEW} writes {VIEW}.ts: {e}"))
}

#[test]
fn the_generated_type_carries_the_decision() {
    let ts = exported();
    assert!(
        ts.contains("decision: ApprovalDecisionDto | null"),
        "the item's decision:\n{ts}"
    );
    for field in [
        "approverId: string",
        "outcome: string",
        "reason: string",
        "at: string",
    ] {
        assert!(ts.contains(field), "ApprovalDecisionDto has {field}:\n{ts}");
    }
}

#[test]
fn the_generated_file_is_committed() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../web/src/shared/api/generated/{VIEW}.ts"));
    let committed = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("web/src/shared/api/generated/{VIEW}.ts: {e}"));
    assert_eq!(committed, exported(), "{VIEW}.ts is what the DTOs generate");
}
