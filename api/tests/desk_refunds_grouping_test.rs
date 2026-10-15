//! Ticket #1933 — `desk_refunds` grouping test (§9): two approved refunds of two originals with
//! different receipts, lines and payments each carry only their own original's receipt, lines (by
//! `line_no`) and payments (by `payment_no`).

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::TestDb;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const ME: &str = "staff:FRONT:MAIN";
const VIEW: &str = "/api/v3/views/desk_refunds";

const COMPLETED_AT: &str = "2026-10-06T05:00:00Z";
const REQUESTED_AT_A: &str = "2026-10-06T06:00:00Z";
const DECIDED_AT_A: &str = "2026-10-06T06:30:00Z";
const REQUESTED_AT_B: &str = "2026-10-06T06:05:00Z";
const DECIDED_AT_B: &str = "2026-10-06T06:35:00Z";
const TODAY: &str = "(now() AT TIME ZONE 'Asia/Bangkok')::date";

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "spec {spec:?}");
    let role = Role::parse(parts.next().expect("a role")).expect("a known role");
    let branch = parts.next().expect("a branch");
    Sessions {
        owner: None,
        staff: Actor::staff(
            format!("employee-{}", role.as_str()),
            role,
            BranchId::new(branch),
        ),
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

async fn get_rows(db: &TestDb, who: &str, branch: &str) -> Vec<Value> {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(&format!("{VIEW}?branch={branch}"))
        .insert_header((header::HOST, "xc.test"))
        .insert_header((AS, who))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).expect("valid json response");
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("data.rows is an array: {body}"))
        .clone()
}

async fn prepare(db: &TestDb) {
    db.exec(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CU-GROUP-MAIN', organization_id, 'CU-GROUP-MAIN', 'Somchai', 'DEMO', now() \
         FROM branches WHERE id = 'MAIN'",
    )
    .await;
    for key in ["RECEIPT-MAIN-A", "RECEIPT-MAIN-B"] {
        db.exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('{key}', 'MAIN', 'owner', 'text/html', 'sha-{key}', now())"
        ))
        .await;
    }
}

async fn seed_original(
    db: &TestDb,
    number: &str,
    receipt_file_key: &str,
    lines: &[(i32, &str, &str, &str, &str)],
    payments: &[(i32, &str, &str)],
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let org = "(SELECT organization_id FROM branches WHERE id = 'MAIN')";
    db.exec(&format!(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, submission_id, payload_hash, \
            customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
            bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
            value_date, paid_at, recorded_at, recorded_by, status, completed_at, organization_id) \
         VALUES ('{id}', '{number}', 'MAIN', 'DEMO', 'sub-{id}', 'hash', 'CU-GROUP-MAIN', \
            'Somchai', 'THA', 'BUY', 'USD', 100, 32.1, 3210.00, 10.00, 0, 0, 3200.00, 'Cash', '', \
            '', 'EMP-001', 'WALK-IN', '', '2026-10-06', '{COMPLETED_AT}', '{COMPLETED_AT}', \
            'owner', 'COMPLETED', '{COMPLETED_AT}', {org})"
    ))
    .await;
    for (line_no, side, currency, quantity, settle) in lines {
        db.exec(&format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, \
                created_at, updated_at) \
             VALUES (gen_random_uuid(), {org}, '{id}', '{side}', '{currency}', {quantity}, 1, 32.1, \
                {settle}, 0, {settle}, {line_no}, '{COMPLETED_AT}', '{COMPLETED_AT}')"
        ))
        .await;
    }
    for (payment_no, method, amount) in payments {
        db.exec(&format!(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
                reference, account, slip_file_key, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
             VALUES (gen_random_uuid(), {org}, '{id}', {payment_no}, '{method}', '', '', \
                NULL, {amount}, 0, '{COMPLETED_AT}', '{COMPLETED_AT}', '{COMPLETED_AT}')"
        ))
        .await;
    }
    db.exec(&format!(
        "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
            document_ref, kind, created_at, updated_at) \
         VALUES (gen_random_uuid(), {org}, '{id}', '{number}', 1, '{COMPLETED_AT}', \
            '{receipt_file_key}', 'RECEIPT', '{COMPLETED_AT}', '{COMPLETED_AT}')"
    ))
    .await;
    id
}

async fn seed_approved_reversal(
    db: &TestDb,
    original: &str,
    reason: &str,
    amount: &str,
    requested_at: &str,
    decided_at: &str,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let request = uuid::Uuid::new_v4().to_string();
    let org = "(SELECT organization_id FROM branches WHERE id = 'MAIN')";
    db.exec(&format!(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
            object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
            status, idempotency_key) \
         VALUES ('{request}', {org}, 'TRANSACTION_REVERSAL', 'MAIN', '{id}', 'h', '1', \
            '{{}}', 'EMP-FRONT', '{requested_at}', 'APPROVED', 'k-{request}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO approval_decisions (id, request_id, approver_id, approver_role, outcome, \
            reason, at) \
         VALUES (gen_random_uuid(), '{request}', 'EMP-FINANCE', 'FINANCE', 'APPROVED', \
            'checked', '{decided_at}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
            reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
            requested_by, requested_at, posted_by, posted_at) \
         VALUES ('{id}', {org}, 'MAIN', '{original}', 'APPROVED', '{reason}', {amount}, \
            {TODAY}, '{request}', NULL, NULL, 'EMP-FRONT', '{requested_at}', NULL, NULL)"
    ))
    .await;
    id
}

#[tokio::test]
async fn two_approved_refunds_each_keep_only_their_own_receipt_lines_and_payments() {
    let db = TestDb::start().await;
    prepare(&db).await;

    let orig_a = seed_original(
        &db,
        "TEST-XC-MAIN-000101",
        "RECEIPT-MAIN-A",
        &[
            (2, "BUY", "EUR", "5.5", "200.00"),
            (1, "BUY", "USD", "100", "3000.00"),
        ],
        &[(2, "PromptPay", "200.00"), (1, "Cash", "3000.00")],
    )
    .await;
    let rev_a = seed_approved_reversal(
        &db,
        &orig_a,
        "wrong currency A",
        "3200.00",
        REQUESTED_AT_A,
        DECIDED_AT_A,
    )
    .await;

    let orig_b = seed_original(
        &db,
        "TEST-XC-MAIN-000202",
        "RECEIPT-MAIN-B",
        &[
            (2, "SELL", "GBP", "50", "2200.00"),
            (1, "SELL", "JPY", "10000", "2400.00"),
        ],
        &[(2, "Cash", "600.00"), (1, "Bank Transfer", "4000.00")],
    )
    .await;
    let rev_b = seed_approved_reversal(
        &db,
        &orig_b,
        "wrong amount B",
        "4600.00",
        REQUESTED_AT_B,
        DECIDED_AT_B,
    )
    .await;

    let rows = get_rows(&db, ME, "MAIN").await;
    assert_eq!(rows.len(), 2, "two approved refunds: {rows:?}");

    let row_a = rows
        .iter()
        .find(|r| r["reversal"]["id"] == json!(rev_a))
        .unwrap_or_else(|| panic!("missing rev_a in {rows:?}"));
    assert_eq!(row_a["reversal"]["originalTransactionId"], json!(orig_a));
    assert_eq!(row_a["reversal"]["reason"], json!("wrong currency A"));
    assert_eq!(row_a["reversal"]["amount"], json!("3200.00"));
    assert_eq!(row_a["receiptNumber"], json!("TEST-XC-MAIN-000101"));
    assert_eq!(
        row_a["lines"],
        json!([
            {
                "shopSide": "BUY",
                "currency": "USD",
                "quantity": "100.0000",
                "settleThb": "3000.00",
            },
            {
                "shopSide": "BUY",
                "currency": "EUR",
                "quantity": "5.5000",
                "settleThb": "200.00",
            },
        ])
    );
    assert_eq!(
        row_a["payments"],
        json!([
            { "method": "Cash", "amountThb": "3000.00" },
            { "method": "PromptPay", "amountThb": "200.00" },
        ])
    );

    let row_b = rows
        .iter()
        .find(|r| r["reversal"]["id"] == json!(rev_b))
        .unwrap_or_else(|| panic!("missing rev_b in {rows:?}"));
    assert_eq!(row_b["reversal"]["originalTransactionId"], json!(orig_b));
    assert_eq!(row_b["reversal"]["reason"], json!("wrong amount B"));
    assert_eq!(row_b["reversal"]["amount"], json!("4600.00"));
    assert_eq!(row_b["receiptNumber"], json!("TEST-XC-MAIN-000202"));
    assert_eq!(
        row_b["lines"],
        json!([
            {
                "shopSide": "SELL",
                "currency": "JPY",
                "quantity": "10000.0000",
                "settleThb": "2400.00",
            },
            {
                "shopSide": "SELL",
                "currency": "GBP",
                "quantity": "50.0000",
                "settleThb": "2200.00",
            },
        ])
    );
    assert_eq!(
        row_b["payments"],
        json!([
            { "method": "Bank Transfer", "amountThb": "4000.00" },
            { "method": "Cash", "amountThb": "600.00" },
        ])
    );
}
