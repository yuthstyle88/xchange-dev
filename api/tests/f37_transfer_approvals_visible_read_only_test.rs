//! R4 / F37 (#564): an over-limit cash transfer is visible, read-only, to FINANCE and BRANCH_MANAGER.
//! Written before the implementation by the ticket author (claude/33691c06). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Owner ruling R4 (B) on #564, issuecomment-6673699296 (accepted 2026-11-06 07:56 ICT):
//! "FINANCE and BRANCH_MANAGER see over-limit rows read-only, with no decide button."
//! Symptom (#564 issuecomment-6657891388, F37): two USD 600 cash-transfer requests were over the
//! limit; finance and the branch manager saw no pending request, only executive and owner did.
//!
//! The rule pinned here (ticket author's design):
//! - every row of `GET /api/v3/views/approval_requests` carries `canDecide: bool`;
//! - a `CASH_TRANSFER` row is listed to a caller who holds `ApprovalsDecide` on the transfer's
//!   `from_branch`, with `canDecide = TransferApprovalPolicy::may_decide(..).is_ok()`;
//! - every other type keeps "listed iff `may_decide`" (`approvals_read_per_type_test.rs`), so each
//!   listed row of another type has `canDecide = true`;
//! - `approvalDecide` still re-checks `may_decide`: seeing a row does not let FINANCE or a
//!   BRANCH_MANAGER decide an over-limit transfer.
//!
//! Rows are read as JSON (`row["canDecide"]`), so this file compiles before the field exists.
//! Needs `TEST_DATABASE_URL`; nothing here skips. Callers come from the `x-test-as` header:
//! `owner` (organisation-wide EXECUTIVE + ADMIN) or `staff:<ROLE>:<branch>`, ROLE one of
//! `test_callers::V2_STAFF` (`MANAGER` = BRANCH_MANAGER, `FINANCE` = FINANCE).

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::{TestDb, unique};
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

/// The over-limit request (the F37 USD 600 transfer).
const ABOVE: &str = "00000000-0000-4000-8000-0000000000a1";
/// A within-limit request of the same branch.
const WITHIN: &str = "00000000-0000-4000-8000-0000000000b1";
/// A rate publication: a type that keeps "listed iff may_decide".
const RATE: &str = "00000000-0000-4000-8000-0000000000c1";

const OTHER_BRANCH: &str = "XC-F37-B2";

struct HeaderCallers;

fn callers_of(spec: &str) -> Callers {
    if spec == "owner" {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "spec {spec:?}");
    let role_name = parts.next().expect("a role");
    let roles = test_callers::v2_staff_roles(role_name)
        .unwrap_or_else(|| panic!("unknown role {role_name:?}"));
    let branch = parts.next().expect("a branch");
    test_callers::callers(
        None,
        Some(test_callers::staff(
            &format!("employee-{role_name}-{branch}"),
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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(db: &TestDb, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
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
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("f37-key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The rows `who` is listed at `query` (e.g. `""` or `"?branch=MAIN"`); panics unless 200.
async fn rows(db: &TestDb, who: &str, query: &str) -> Vec<Value> {
    let a = call(
        db,
        who,
        Method::GET,
        &format!("/api/v3/views/approval_requests{query}"),
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "{who}: {}", a.text);
    a.body
        .pointer("/data/requests")
        .or_else(|| a.body.pointer("/requests"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{who}: no requests list in {}", a.text))
        .clone()
}

/// The row `id` among `rows`, if listed.
fn row<'a>(rows: &'a [Value], id: &str) -> Option<&'a Value> {
    rows.iter().find(|r| r["id"] == json!(id))
}

/// `(listed, canDecide)` of request `id` for `who` on their own session branch.
async fn seen(db: &TestDb, who: &str, id: &str) -> (bool, Value) {
    let listed = rows(db, who, "").await;
    match row(&listed, id) {
        Some(r) => (true, r["canDecide"].clone()),
        None => (false, Value::Null),
    }
}

/// A PENDING `CASH_TRANSFER` request at MAIN, from MAIN, as `TransferFlow::submit_in` writes it
/// (`transfer_flow.rs`: payload `{transfer_id, from_branch, within_limit}`, branch = from_branch).
async fn transfer_request(db: &TestDb, id: &str, within_limit: bool) {
    db.exec(&format!(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
            policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES ('{id}', 'CASH_TRANSFER', 'MAIN', '{id}', 'DRAFT', '1', \
            '{{\"transfer_id\": \"{id}\", \"from_branch\": \"MAIN\", \"within_limit\": {within_limit}}}', \
            'teller-who-asked', now(), 'PENDING', 'k-{id}', '00000000-0000-0000-0000-000000000001')"
    ))
    .await;
}

async fn rate_request(db: &TestDb, id: &str) {
    db.exec(&format!(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
            policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES ('{id}', 'RATE_PUBLICATION', 'MAIN', 'obj-{id}', 'h', '1', '{{}}', \
            'someone', now(), 'PENDING', 'k-{id}', '00000000-0000-0000-0000-000000000001')"
    ))
    .await;
}

async fn seeded() -> TestDb {
    let db = TestDb::start().await;
    db.branch(OTHER_BRANCH).await;
    transfer_request(&db, ABOVE, false).await;
    transfer_request(&db, WITHIN, true).await;
    rate_request(&db, RATE).await;
    db
}

/// F37 itself: FINANCE of MAIN now sees the over-limit transfer, with no right to decide it.
#[tokio::test]
async fn finance_sees_an_over_limit_transfer_read_only() {
    let db = seeded().await;
    assert_eq!(
        seen(&db, "staff:FINANCE:MAIN", ABOVE).await,
        (true, json!(false)),
        "R4 (B): FINANCE sees the over-limit row read-only (listed, canDecide false)"
    );
    db.finish().await;
}

/// FINANCE holds no transfer approval kind at all, so a within-limit transfer is read-only too.
#[tokio::test]
async fn finance_sees_a_within_limit_transfer_read_only() {
    let db = seeded().await;
    assert_eq!(
        seen(&db, "staff:FINANCE:MAIN", WITHIN).await,
        (true, json!(false)),
        "FINANCE holds ApprovalsDecide on MAIN but not ApproveCashTransferWithinLimit"
    );
    db.finish().await;
}

/// F37 itself: the BRANCH_MANAGER of the source branch sees the over-limit transfer read-only.
#[tokio::test]
async fn the_source_branch_manager_sees_an_over_limit_transfer_read_only() {
    let db = seeded().await;
    assert_eq!(
        seen(&db, "staff:MANAGER:MAIN", ABOVE).await,
        (true, json!(false)),
        "R4 (B): BRANCH_MANAGER of MAIN sees the over-limit row read-only (listed, canDecide false)"
    );
    db.finish().await;
}

/// The within-limit transfer stays the source branch manager's to decide.
#[tokio::test]
async fn the_source_branch_manager_decides_a_within_limit_transfer() {
    let db = seeded().await;
    assert_eq!(
        seen(&db, "staff:MANAGER:MAIN", WITHIN).await,
        (true, json!(true)),
        "BRANCH_MANAGER of MAIN holds ApproveCashTransferWithinLimit on MAIN"
    );
    db.finish().await;
}

/// The executive (the owner) still decides the over-limit transfer.
#[tokio::test]
async fn the_owner_decides_an_over_limit_transfer() {
    let db = seeded().await;
    assert_eq!(
        seen(&db, "owner", ABOVE).await,
        (true, json!(true)),
        "EXECUTIVE holds ApproveCashTransferAboveLimit organisation-wide"
    );
    db.finish().await;
}

/// The same rule read the other way: the owner holds `ApprovalsDecide` on MAIN but not
/// `ApproveCashTransferWithinLimit` (`role_grants.rs`, EXECUTIVE), so a within-limit transfer is
/// listed to the owner read-only. No special case for the owner.
#[tokio::test]
async fn the_owner_sees_a_within_limit_transfer_read_only() {
    let db = seeded().await;
    assert_eq!(
        seen(&db, "owner", WITHIN).await,
        (true, json!(false)),
        "listed because the owner holds ApprovalsDecide on MAIN; read-only because may_decide refuses"
    );
    db.finish().await;
}

/// Visibility follows `ApprovalsDecide` on the transfer's `from_branch`: a branch manager of
/// another branch sees neither transfer, on its own branch or by naming MAIN.
#[tokio::test]
async fn a_branch_manager_of_another_branch_does_not_see_the_transfers() {
    let db = seeded().await;
    let who = format!("staff:MANAGER:{OTHER_BRANCH}");
    let own = rows(&db, &who, "").await;
    assert!(
        row(&own, ABOVE).is_none() && row(&own, WITHIN).is_none(),
        "{who} on its own branch: {own:?}"
    );
    let named = call(
        &db,
        &who,
        Method::GET,
        "/api/v3/views/approval_requests?branch=MAIN",
        None,
    )
    .await;
    if named.status == StatusCode::OK {
        let listed = named
            .body
            .pointer("/data/requests")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            row(&listed, ABOVE).is_none() && row(&listed, WITHIN).is_none(),
            "{who} naming MAIN must not see MAIN's transfers: {}",
            named.text
        );
    }
    db.finish().await;
}

/// Other types are unchanged: listed iff `may_decide`, so every listed row of them can be decided.
#[tokio::test]
async fn a_listed_row_of_another_type_can_be_decided() {
    let db = seeded().await;
    assert_eq!(
        seen(&db, "owner", RATE).await,
        (true, json!(true)),
        "the owner decides a rate publication"
    );
    for who in ["staff:FINANCE:MAIN", "staff:MANAGER:MAIN"] {
        assert_eq!(
            seen(&db, who, RATE).await,
            (false, Value::Null),
            "{who}: RATE_PUBLICATION keeps listed-iff-may_decide (#874), so it is not listed"
        );
    }
    db.finish().await;
}

/// Every listed row carries a boolean `canDecide`, whoever reads.
#[tokio::test]
async fn every_listed_row_carries_a_boolean_can_decide() {
    let db = seeded().await;
    for who in ["owner", "staff:FINANCE:MAIN", "staff:MANAGER:MAIN"] {
        for r in rows(&db, who, "").await {
            assert!(
                r["canDecide"].is_boolean(),
                "{who}: row {} has no boolean canDecide: {r}",
                r["id"]
            );
        }
    }
    db.finish().await;
}

/// Seeing is not deciding: `approvalDecide` still refuses FINANCE and the source branch manager
/// on the over-limit transfer (`ApprovalService::decide` re-checks `may_decide`), and the request
/// stays PENDING with no decision row.
#[tokio::test]
async fn finance_and_the_branch_manager_still_cannot_decide_an_over_limit_transfer() {
    let db = seeded().await;
    for who in ["staff:FINANCE:MAIN", "staff:MANAGER:MAIN"] {
        let a = call(
            &db,
            who,
            Method::POST,
            &format!("/api/approvals/{ABOVE}/decide"),
            Some(json!({ "outcome": "APPROVED", "reason": "F37 read-only check" })),
        )
        .await;
        assert_eq!(a.status, StatusCode::FORBIDDEN, "{who}: {}", a.text);
        assert!(
            a.text.contains("ABOVE_LIMIT"),
            "{who}: refused by TransferApprovalPolicy (ABOVE_LIMIT): {}",
            a.text
        );
    }
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM approval_requests WHERE id = '{ABOVE}'"
        ))
        .await
        .as_deref(),
        Some("PENDING")
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM approval_decisions WHERE request_id = '{ABOVE}'"
        ))
        .await,
        0
    );
    db.finish().await;
}
