//! Ticket #874 — the approval queue is read per type: `GET /api/v3/views/approval_requests` lists
//! only the requests whose type's own `ApprovalPolicy::may_decide` admits the reader.
//! Written before the implementation by the ticket author (claude/f03345fe). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Lane controller's ruling (#621 issuecomment-6093189074, 2026-10-10):
//! - BRANCH_MANAGER and FINANCE (RG4 of #682) qualify for `ApprovalsDecide`, which also lets it pass the gate of
//!   `approvalRead` (`policy_route_table_test.rs:105` maps both actions to that permission);
//! - **the read is per type.** A rate publication stays OWNER/EXECUTIVE-only
//!   (`RatePublicationPolicy::may_decide`), so a manager must not see one in the list.
//!
//! The rule pinned here: a row is listed when the registered policy of its `type` says `Ok` to
//! `may_decide(reader, row)`. A row whose type is not registered is never listed. The branch and
//! the `status`/`type` filters are unchanged.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Sessions come from the `x-test-as` header, as in
//! `v3_rate_engine_routes_test.rs` (`owner`, `staff:<ROLE>:<branch>`).
//!
//! Signatures: none new on the wire. The view's DTO is unchanged.

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
async fn request(db: &TestDb, type_name: &str) {
    db.exec(&format!(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
            policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), '{type_name}', 'MAIN', 'obj-{type_name}', 'h', '1', '{{}}', \
            'someone', now(), 'PENDING', 'k-{type_name}')"
    ))
    .await;
}

#[tokio::test]
async fn a_branch_manager_does_not_see_the_rate_approvals_the_owner_sees() {
    let db = TestDb::start().await;
    request(&db, "RATE_PUBLICATION").await;

    assert_eq!(
        listed(&db, "owner").await,
        ["RATE_PUBLICATION"],
        "the owner, who decides rate publications, sees it"
    );
    for role in ["MANAGER", "BACKEND", "FINANCE"] {
        assert_eq!(
            listed(&db, &format!("staff:{role}:MAIN")).await,
            Vec::<String>::new(),
            "{role}: passes the gate (#874) but may not decide a rate publication, so it is not listed"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_request_of_a_type_nobody_registered_is_never_listed() {
    let db = TestDb::start().await;
    request(&db, "RATE_PUBLICATION").await;
    request(&db, "NO_SUCH_TYPE").await;
    assert_eq!(
        listed(&db, "owner").await,
        ["RATE_PUBLICATION"],
        "a type without a registered policy is nobody's to decide"
    );
    db.finish().await;
}
