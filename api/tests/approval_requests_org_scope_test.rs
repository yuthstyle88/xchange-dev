//! Ticket #2694 — organisation scope for approval_requests view:
//! `GET /api/v3/views/approval_requests` lists only requests matching the named branch's organisation.
//!
//! Behaviour table (§8):
//! - A1: organisation B (second org) has an org-wide PENDING request (branch_id NULL, organization_id = B);
//!   EXECUTIVE@* of default org reads `approval_requests?branch=XC-B01` -> B's request not listed (main: listed).
//! - A2: same caller; the default org has its own org-wide request -> listed (kept).
//! - A3: same caller; a branch-bound request of XC-B01 -> listed (kept).
//! - A4: BRANCH_MANAGER@XC-B01 and a rate-publication request -> not listed (kept: per-type rule, #874).

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::TestDb;
use serde_json::Value;
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const ORG_A: &str = "00000000-0000-0000-0000-000000000001";
const ORG_B: &str = "00000000-0000-0000-0000-000000000099";

struct HeaderCallers;

fn callers_of(spec: &str) -> Callers {
    if spec == "owner" {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "spec {spec:?}");
    let role_name = parts.next().expect("a role");
    let roles: &[Role] = match role_name {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown role {other:?}"),
    };
    let branch = parts.next().expect("a branch");
    test_callers::callers(
        None,
        Some(test_callers::staff(
            &format!("employee-{role_name}"),
            roles,
            BranchId::new(branch),
        )),
    )
}

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
    }
}

/// The listed requests' `type`s, in the view's order; panics unless the view answers 200.
async fn listed(db: &TestDb, who: &str) -> Vec<String> {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri("/api/v3/views/approval_requests")
        .insert_header((header::HOST, "xc.test"))
        .insert_header((AS, who))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "{who}: {text}");
    let body: Value = serde_json::from_slice(&bytes).expect("JSON");
    let requests = body
        .pointer("/data/requests")
        .or_else(|| body.pointer("/requests"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{who}: no requests list in {text}"));
    requests
        .iter()
        .map(|r| r["type"].as_str().expect("a type").to_owned())
        .collect()
}

/// A PENDING request of `type_name` at MAIN.
#[allow(dead_code)]
async fn request(db: &TestDb, type_name: &str) {
    db.exec(&format!(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
            policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), '{type_name}', 'MAIN', 'obj-{type_name}', 'h', '1', '{{}}', \
            'someone', now(), 'PENDING', 'k-{type_name}')"
    ))
    .await;
}

/// The listed requests' `type`s at `uri`, in the view's order; panics unless the view answers 200.
async fn listed_at(db: &TestDb, who: &str, uri: &str) -> Vec<String> {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"))
        .insert_header((AS, who))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "{who}: {text}");
    let body: Value = serde_json::from_slice(&bytes).expect("JSON");
    let requests = body
        .pointer("/data/requests")
        .or_else(|| body.pointer("/requests"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{who}: no requests list in {text}"));
    requests
        .iter()
        .map(|r| r["type"].as_str().expect("a type").to_owned())
        .collect()
}

async fn setup_orgs_and_branches(db: &TestDb) {
    db.exec(&format!(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ('{ORG_B}', 'ORG2', 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING"
    ))
    .await;

    db.exec(&format!(
        "INSERT INTO branches (id, name, organization_id, code) \
         VALUES ('XC-B01', 'B01', '{ORG_A}', 'XC-B01') \
         ON CONFLICT (id) DO NOTHING"
    ))
    .await;

    db.exec(&format!(
        "INSERT INTO branches (id, name, organization_id, code) \
         VALUES ('ORG2-B01', 'ORG2 Branch', '{ORG_B}', 'ORG2-B01') \
         ON CONFLICT (id) DO NOTHING"
    ))
    .await;
}

async fn request_at(
    db: &TestDb,
    type_name: &str,
    branch_id: Option<&str>,
    org_id: &str,
    suffix: &str,
) {
    let branch_val = match branch_id {
        Some(b) => format!("'{b}'"),
        None => "NULL".to_string(),
    };
    db.exec(&format!(
        "INSERT INTO approval_requests (id, type, branch_id, organization_id, object_id, object_version_hash, \
            policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), '{type_name}', {branch_val}, '{org_id}', 'obj-{type_name}-{suffix}', 'h', '1', '{{}}', \
            'someone', now(), 'PENDING', 'k-{type_name}-{suffix}')"
    ))
    .await;
}

#[tokio::test]
async fn a1_org_b_org_wide_request_not_listed_for_org_a_executive() {
    let db = TestDb::start().await;
    setup_orgs_and_branches(&db).await;

    // Organisation B has an org-wide PENDING request (branch_id NULL, organization_id = B)
    request_at(&db, "RATE_PUBLICATION", None, ORG_B, "b-org-wide").await;

    // EXECUTIVE@* (owner) of the default org reads approval_requests?branch=XC-B01
    let rows = listed_at(
        &db,
        "owner",
        "/api/v3/views/approval_requests?branch=XC-B01",
    )
    .await;
    assert_eq!(
        rows,
        Vec::<String>::new(),
        "organisation B's org-wide request must not leak to organisation A"
    );

    db.finish().await;
}

#[tokio::test]
async fn a2_default_org_org_wide_request_listed_for_org_a_executive() {
    let db = TestDb::start().await;
    setup_orgs_and_branches(&db).await;

    // Default org has its own org-wide request
    request_at(&db, "RATE_PUBLICATION", None, ORG_A, "a-org-wide").await;

    let rows = listed_at(
        &db,
        "owner",
        "/api/v3/views/approval_requests?branch=XC-B01",
    )
    .await;
    assert_eq!(
        rows,
        ["RATE_PUBLICATION"],
        "organisation A's org-wide request is listed"
    );

    db.finish().await;
}

#[tokio::test]
async fn a3_branch_bound_request_of_named_branch_listed_for_org_a_executive() {
    let db = TestDb::start().await;
    setup_orgs_and_branches(&db).await;

    // Default org has a branch-bound request of XC-B01
    request_at(&db, "RATE_PUBLICATION", Some("XC-B01"), ORG_A, "a-b01").await;

    let rows = listed_at(
        &db,
        "owner",
        "/api/v3/views/approval_requests?branch=XC-B01",
    )
    .await;
    assert_eq!(
        rows,
        ["RATE_PUBLICATION"],
        "branch-bound request of XC-B01 is listed"
    );

    db.finish().await;
}

#[tokio::test]
async fn a4_finance_does_not_see_rate_publication_request() {
    let db = TestDb::start().await;
    setup_orgs_and_branches(&db).await;

    // A rate-publication request of XC-B01
    request_at(&db, "RATE_PUBLICATION", Some("XC-B01"), ORG_A, "rate-pub").await;

    // Under §11.2, FINANCE holds `ApprovalsDecide` (passes the route gate) while BRANCH_MANAGER does not.
    let rows = listed_at(
        &db,
        "staff:FINANCE:XC-B01",
        "/api/v3/views/approval_requests?branch=XC-B01",
    )
    .await;
    assert_eq!(
        rows,
        Vec::<String>::new(),
        "FINANCE may not decide rate publications, per-type rule #874"
    );

    let rows_direct = listed(&db, "staff:FINANCE:XC-B01").await;
    assert_eq!(
        rows_direct,
        Vec::<String>::new(),
        "FINANCE direct view read also empty"
    );

    db.finish().await;
}
