//! G1-BE #5914 (staff rerun 2026-11-02, G1; design `docs/design/blueprint/staff-e2e-1101-reads.md` §2 as amended by
//! PR #5913): every decision of the `approval_requests` view carries `approverName`, the `users.name` of
//! `approval_decisions.approver_id`. A blank name and an approver id that is no users row (a policy decision's
//! `policy:…`) answer `null`. The join adds or drops no item.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL`; the database tests do not skip.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::TestDb;
use serde_json::Value;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

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
                policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'RATE_PUBLICATION', 'MAIN', '{obj}', 'h', '1', '{{}}', \
                'proposer-1', '{at}', '{status}', 'k-{obj}', '00000000-0000-0000-0000-000000000001') RETURNING id::text"
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

/// A `users` row of org XC named `name`; its id.
async fn user(db: &TestDb, name: &str) -> String {
    db.text(&format!(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ('00000000-0000-0000-0000-000000000001', '{name}', 'ACTIVE', 'test') RETURNING id::text"
    ))
    .await
    .expect("a user id")
}

#[tokio::test]
async fn g1_a_decision_by_a_user_names_the_approver() {
    let db = TestDb::start().await;
    let somchai = user(&db, "Somchai Finance").await;
    let id = request(&db, "obj-named", "APPROVED", "2026-11-02T01:00:00Z").await;
    decision(&db, &id, &somchai, "APPROVED", "ok", "2026-11-02T01:05:00Z").await;

    let items = listed(&db).await;
    let d = &item(&items, "obj-named")["decision"];
    assert_eq!(d["approverName"], "Somchai Finance", "{d}");
    assert_eq!(
        d["approverId"],
        somchai.as_str(),
        "approverId stays the stored id: {d}"
    );
    db.finish().await;
}

#[tokio::test]
async fn g1_a_blank_name_a_policy_decision_and_a_non_user_id_answer_null() {
    let db = TestDb::start().await;
    let blank = user(&db, "   ").await;
    let a = request(&db, "obj-blank", "APPROVED", "2026-11-02T01:00:00Z").await;
    decision(&db, &a, &blank, "APPROVED", "ok", "2026-11-02T01:05:00Z").await;
    let b = request(&db, "obj-policy", "APPROVED", "2026-11-02T01:10:00Z").await;
    decision(
        &db,
        &b,
        "policy:1",
        "APPROVED",
        "auto",
        "2026-11-02T01:15:00Z",
    )
    .await;
    let c = request(&db, "obj-legacy", "REJECTED", "2026-11-02T01:20:00Z").await;
    decision(&db, &c, "owner-9", "REJECTED", "no", "2026-11-02T01:25:00Z").await;
    request(&db, "obj-open", "PENDING", "2026-11-02T01:30:00Z").await;

    let items = listed(&db).await;
    assert_eq!(items.len(), 4, "the join adds or drops no item: {items:?}");
    for obj in ["obj-blank", "obj-policy", "obj-legacy"] {
        let d = &item(&items, obj)["decision"];
        assert!(
            d.as_object()
                .expect("a decision object")
                .contains_key("approverName"),
            "{obj}: approverName is always present: {d}"
        );
        assert_eq!(d["approverName"], Value::Null, "{obj}: {d}");
    }
    assert_eq!(
        item(&items, "obj-policy")["decision"]["approverId"],
        "policy:1"
    );
    assert_eq!(
        item(&items, "obj-open")["decision"],
        Value::Null,
        "no decision row: null as today"
    );
    db.finish().await;
}
