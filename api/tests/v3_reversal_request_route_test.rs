//! Ticket #682 (RV-1+RV-2), Part 2 (RV-2) — `POST /api/transactions/{id}/reversal-requests`, the
//! command over `RequestReversal::run_in`: the route gate, the answer, the refusals and the
//! idempotency key.
//! Written before the implementation by the ticket author for the TX/RV chain (claude/f03345fe, on
//! the lane controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/reversal.md` §4.2 (§10's route; the rates §6 command seam with
//! `Idempotency-Key`; input `{ reason }`), §8.1 RV-2. **It needs the route row `(Rpc,
//! "reversalRequest")` that #874 adds** (finding RG3, lane controller 2026-10-10), so it passes only
//! once #874 and this ticket have both merged.
//!
//! Signatures and shapes this test assumes ([required] in ticket §7/§8):
//! - the command file is **flat**: `api/src/presentation/http/v3/commands/reversal_requests.rs`.
//!   `api/build.rs` serves only plain files in `commands/` (#672's 2026-10-09 amendment), so the
//!   design's `commands/exchange/reversals.rs` would never be served (finding RG7). It has
//!   `ROUTE = "/api/transactions/{id}/reversal-requests"`, `METHOD = POST`, and `INHERITS = (Rpc,
//!   "reversalRequest")`;
//! - the answer (camelCase, money as a 2-place string): `{ id, status, originalTransactionId,
//!   reason, amountThb, businessDate, approvalRequestId }`. Command DTOs commit no TS, so the
//!   design's `reversal.ts` is not written (RG7);
//! - refusal statuses, as `approval_decide.rs` maps them: `validationFailed` 400, `outOfScope` 403,
//!   `staleVersion` 409, `reversalWindowClosed` 409, `idempotencyConflict` 409.
//!
//! The command runs on the server clock, so the originals are completed `now()` (inside the window)
//! or two days ago (outside it). MAIN's timezone is OR-1's default, Asia/Bangkok.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Callers come from the `x-test-as` header
//! (`owner`, `staff:<ROLE>:<branch>`), as in `v3_rate_engine_routes_test.rs`.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::TestDb;
use serde_json::{Value, json};
use xc_auth::domain::access::{Permission, RoleGrants};
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
    let role = parts.next().expect("a role");
    let roles = test_callers::v2_staff_roles(role).unwrap_or_else(|| panic!("unknown role {role}"));
    let branch = parts.next().expect("a branch");
    test_callers::callers(
        None,
        Some(test_callers::staff(
            &format!("employee-{role}"),
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

/// `POST /api/transactions/{id}/reversal-requests` as `who` with `key`: (status, body).
async fn post(db: &TestDb, who: &str, id: &str, body: Value, key: &str) -> (StatusCode, Value) {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(&format!("/api/transactions/{id}/reversal-requests"))
        .insert_header((header::HOST, "xc.test"))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", "http://xc.test"))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string())
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

async fn seed(db: &TestDb) {
    db.exec(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CU-1', organization_id, 'CU-1', 'Somchai', 'LIVE', now() FROM branches WHERE id = 'MAIN'",
    )
    .await;
    db.exec(
        "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, 'Teller', 'ACTIVE', 'test' FROM branches WHERE id = 'MAIN' \
             RETURNING id) \
         INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         SELECT 'SH-1', 'MAIN', 'OPEN', now(), 'owner', 'open-1', u.id FROM u",
    )
    .await;
}

/// A LIVE COMPLETED BUY at MAIN settling 3,210.00 THB (fee 10), completed at `completed` (SQL).
async fn completed(db: &TestDb, n: u32, completed: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    db.exec(&format!(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
            customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
            recorded_by, status, completed_at) \
         VALUES ('{id}', 'XC-R-{n:06}', 'MAIN', 'LIVE', 'SH-1', 'sub-{id}', 'h', 'CU-1', 'Somchai', 'THA', \
            'EMP-001', 'WALK-IN', '', current_date, {completed}, {completed}, 'owner', 'COMPLETED', \
            {completed})"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
            quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, \
            updated_at) \
         SELECT gen_random_uuid(), organization_id, id, 'BUY', 'USD', 100, 1, 32.1, 3220, 10, 3210, 1, \
            now(), now() \
         FROM transactions WHERE id = '{id}'"
    ))
    .await;
    id
}

#[actix_web::test]
async fn a_teller_requests_a_reversal_and_the_same_key_replays() {
    let db = TestDb::start().await;
    seed(&db).await;
    let tx = completed(&db, 1, "now()").await;

    let (status, body) = post(
        &db,
        "staff:FRONT:MAIN",
        &tx,
        json!({ "reason": "wrong currency" }),
        "k-1",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], json!("REQUESTED"), "{body}");
    assert_eq!(body["originalTransactionId"], json!(tx));
    assert_eq!(body["reason"], json!("wrong currency"));
    assert_eq!(body["amountThb"], json!("3210.00"), "V-1: the settled THB");
    assert!(
        body["id"].is_string() && body["approvalRequestId"].is_string(),
        "{body}"
    );
    let today: String = db
        .text("SELECT (now() AT TIME ZONE 'Asia/Bangkok')::date::text")
        .await
        .unwrap();
    assert_eq!(
        body["businessDate"],
        json!(today),
        "R-1: the date in Bangkok"
    );

    let (again, replay) = post(
        &db,
        "staff:FRONT:MAIN",
        &tx,
        json!({ "reason": "wrong currency" }),
        "k-1",
    )
    .await;
    assert_eq!(
        (again, &replay),
        (StatusCode::OK, &body),
        "the same key replays the answer"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM reversals").await,
        1,
        "written once"
    );

    let (changed, refusal) = post(
        &db,
        "staff:FRONT:MAIN",
        &tx,
        json!({ "reason": "another" }),
        "k-1",
    )
    .await;
    assert_eq!(changed, StatusCode::CONFLICT, "{refusal}");
    assert_eq!(refusal["error"]["code"], json!("idempotencyConflict"));
    db.finish().await;
}

#[actix_web::test]
async fn the_branch_manager_may_request_and_roles_without_reversal_request_may_not() {
    let db = TestDb::start().await;
    seed(&db).await;
    let a = completed(&db, 1, "now()").await;
    let b = completed(&db, 2, "now()").await;
    let (status, body) = post(
        &db,
        "staff:MANAGER:MAIN",
        &a,
        json!({ "reason": "x" }),
        "k-m",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Under §11.2, staff roles whose `RoleGrants::seed` lacks `ReversalRequest` are refused by the route row.
    let refused: Vec<&str> = test_callers::V2_STAFF
        .into_iter()
        .filter_map(|(name, roles)| {
            (!roles
                .iter()
                .any(|&br| RoleGrants::seed(br).contains(&Permission::ReversalRequest)))
            .then_some(name)
        })
        .collect();
    assert!(!refused.is_empty(), "some staff role lacks ReversalRequest");
    for role in refused {
        let who = format!("staff:{role}:MAIN");
        let (status, body) =
            post(&db, &who, &b, json!({ "reason": "x" }), &format!("k-{who}")).await;
        assert!(
            status.is_client_error(),
            "{who}: the route row (#874) refuses: {status} {body}"
        );
    }
    assert_eq!(db.count("SELECT count(*) FROM reversals").await, 1);
    db.finish().await;
}

#[actix_web::test]
async fn the_refusals_answer_their_codes_and_write_nothing() {
    let db = TestDb::start().await;
    seed(&db).await;
    let old = completed(&db, 1, "now() - interval '2 days'").await;
    let ok = completed(&db, 2, "now()").await;
    let (_, first) = post(
        &db,
        "staff:FRONT:MAIN",
        &ok,
        json!({ "reason": "first" }),
        "k-first",
    )
    .await;
    assert_eq!(first["status"], json!("REQUESTED"), "{first}");

    for (what, id, reason, code, status) in [
        (
            "after the business day",
            old.as_str(),
            "x",
            "reversalWindowClosed",
            StatusCode::CONFLICT,
        ),
        (
            "an empty reason",
            ok.as_str(),
            "",
            "validationFailed",
            StatusCode::BAD_REQUEST,
        ),
        (
            "a second live reversal",
            ok.as_str(),
            "again",
            "staleVersion",
            StatusCode::CONFLICT,
        ),
    ] {
        let (got, body) = post(
            &db,
            "staff:FRONT:MAIN",
            id,
            json!({ "reason": reason }),
            &format!("k-{what}"),
        )
        .await;
        assert_eq!(
            (got, &body["error"]["code"]),
            (status, &json!(code)),
            "{what}: {body}"
        );
    }
    let nobody = uuid::Uuid::new_v4().to_string();
    let (got, body) = post(
        &db,
        "staff:FRONT:MAIN",
        &nobody,
        json!({ "reason": "x" }),
        "k-nobody",
    )
    .await;
    assert_eq!(
        (got, &body["error"]["code"]),
        (StatusCode::FORBIDDEN, &json!("outOfScope")),
        "an unknown transaction: {body}"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM reversals").await,
        1,
        "only the first"
    );
    db.finish().await;
}
