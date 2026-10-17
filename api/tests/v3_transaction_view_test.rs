//! Ticket #678 (TX-10) — the B04 view of one exchange: `GET /api/v3/views/transaction?id=` answers
//! the transaction with its state, lines, payment, receipts and approval request.
//! Written before the implementation by the ticket author's successor for the TX chain
//! (claude/f03345fe, on the lane controller's order). Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §4.1 (the states), §4.2 (the view row:
//! "the transaction, lines, payment, receipts, approval state"), §3.2–§3.5 (the columns, after
//! TX-2 #670), §10 (the desk re-reads it while it waits for approval).
//!
//! What this test pins ([required] in ticket §7/§8 unless marked):
//! - the view `transaction` (`NAME = "transaction"`), a `BranchView` with `VERSIONED = false`, filter
//!   `{ id }`. It answers `data = { transaction: TransactionDetailDto | null }`. Its TS is
//!   `web/src/shared/api/generated/transaction.ts`: a view writes only `<its name>.ts`
//!   (`api/tests/v3_types_test.rs:142`);
//! - **`null`, not an error, for an id that is unknown, malformed, or of another branch.** The view
//!   seam turns any load error into a 500 (`api/src/presentation/http/v3/view.rs:476-482`), and the
//!   single-object view `rate_draft` answers `null` the same way (`views/rate_draft.rs:55-68`).
//!   It also never tells a caller that an id exists at another branch. The branch is the one
//!   `Policy::branch` gives: a staff caller's own; for the owner, the `branch` the request names,
//!   else MAIN;
//! - `TransactionDetailDto`, camelCase:
//!   - `id`, `status` (`DRAFT` … `CANCELLED`), `documentRef` (null until COMPLETED), `branchId`,
//!     `customerId`, `quoteId` and `quoteExpiresAt` (both null when no quote);
//!   - `lines`: `[{ lineNo, currency, side, quantity, unitSize, rateSnapshot, thbAmount, feeThb,
//!     settleThb }]`;
//!   - `payment`: `{ method, reference, account, slipFileKey, amountThb, bankFeeThb, paidAt }`;
//!   - `receipts`: `[{ number, revision, issuedAt }]`, by revision;
//!   - `approvalRequest`: `{ id, status } | null`;
//!   - `completedAt`, `cancelledAt`, `cancelReason`: null when not set;
//! - money is a decimal string with 2 places, quantities and unit sizes with 4, rates with 8 (the
//!   stored scale, rates §3.3); instants are RFC 3339 and are compared here as instants;
//! - **authorised by `TransactionRead`** (lane controller's rulings issuecomment-6107277948 and
//!   -6107297093): `INHERITS = (EntryPath::Rpc, "transactionView")`, whose row
//!   `areas/transaction_read.rs` maps to `Need::Permission(TransactionRead)`. TELLER (v3 FRONT) and
//!   BRANCH_MANAGER (v3 MANAGER, BACKEND) read it. A caller with `adminSnapshot`'s permission
//!   (`CompatRead`) but not `TransactionRead` (the owner, RATES, FINANCE) is refused with the code
//!   main gives (`roleNotAllowedHere`, a client error); nobody signed in is refused `signIn`.
//!   No holder of `TransactionRead` is organisation-wide, so the owner's `branch=` cases were dropped
//!   (ruling -6107297093); the content tests read as the branch's MANAGER.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips. Rows are seeded with SQL in TX-2's schema (#670's
//! migration 2, child rows with explicit ids), DEMO (no shift needed). Sessions come from the `x-test-as` header (`owner`,
//! `staff:<ROLE>:<branch>`), as in `v3_rate_engine_routes_test.rs`.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use quote_common::{ORG_1, TestDb};
use serde_json::{Value, json};
use xc_auth::domain::access::Permission;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";

/// The branch's MANAGER (BRANCH_MANAGER): holds `TransactionRead` on MAIN.
const MANAGER: &str = "staff:MANAGER:MAIN";

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    if spec.is_empty() {
        return Sessions {
            owner: None,
            staff: None,
        };
    }
    if spec == "owner" {
        return Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        };
    }
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

/// `GET /api/v3/views/transaction?<query>` as `who`: (status, body).
async fn get(db: &TestDb, who: &str, query: &str) -> (StatusCode, Value) {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::get()
        .uri(&format!("/api/v3/views/transaction?{query}"))
        .insert_header((header::HOST, "xc.test"));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

/// The `transaction` the view answers (`null` included); panics unless 200.
async fn transaction(db: &TestDb, who: &str, query: &str) -> Value {
    let (status, body) = get(db, who, query).await;
    assert_eq!(status, StatusCode::OK, "{who} {query}: {body}");
    body.pointer("/data/transaction")
        .unwrap_or_else(|| panic!("{who} {query}: no data.transaction in {body}"))
        .clone()
}

fn instant(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(v.as_str().unwrap_or_else(|| panic!("an instant, got {v}")))
        .unwrap_or_else(|e| panic!("{v} is RFC 3339: {e}"))
        .with_timezone(&Utc)
}

const T0: &str = "2026-10-06T05:00:00Z";
const T1: &str = "2026-10-06T05:03:00Z";

struct Seeded {
    draft: String,
    pending: String,
    request: String,
    completed: String,
    cancelled: String,
    other_branch: String,
    quote: String,
}

/// A DEMO BUY of 100 USD at 32.1 (fee 10, the shop pays 3200.00) in `status` at `branch`; answers
/// its id. `extra` sets more columns (`name = value` pairs of SQL).
async fn exchange(
    db: &TestDb,
    branch: &str,
    customer: &str,
    status: &str,
    number: Option<&str>,
    extra: &[(&str, &str)],
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let number = number.map_or("NULL".to_owned(), |n| format!("'{n}'"));
    let mut cols = vec![
        "id",
        "document_ref",
        "branch_id",
        "mode",
        "submission_id",
        "payload_hash",
        "customer_id",
        "customer_name",
        "nationality",
        "staff_id",
        "booking_no",
        "notes",
        "value_date",
        "paid_at",
        "recorded_at",
        "recorded_by",
        "status",
        "organization_id",
    ];
    let org = format!("(SELECT organization_id FROM branches WHERE id = '{branch}')");
    let mut vals = vec![
        format!("'{id}'"),
        number,
        format!("'{branch}'"),
        "'DEMO'".into(),
        format!("'sub-{id}'"),
        "'hash'".into(),
        format!("'{customer}'"),
        "'Somchai'".into(),
        "'THA'".into(),
        "'EMP-001'".into(),
        "'WALK-IN'".into(),
        "''".into(),
        "'2026-10-06'".into(),
        format!("'{T0}'"),
        format!("'{T0}'"),
        "'owner'".into(),
        format!("'{status}'"),
        org.clone(),
    ];
    for (c, v) in extra {
        cols.push(c);
        vals.push((*v).to_owned());
    }
    db.exec(&format!(
        "INSERT INTO transactions ({}) VALUES ({})",
        cols.join(", "),
        vals.join(", ")
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
            quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, \
            created_at, updated_at) \
         VALUES (gen_random_uuid(), {org}, '{id}', 'BUY', 'USD', 100, 1, 32.1, 3210.00, 10.00, \
            3200.00, 1, now(), now())"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, \
            method, reference, account, slip_file_key, amount_thb, bank_fee_thb, paid_at, \
            created_at, updated_at) \
         VALUES (gen_random_uuid(), {org}, '{id}', 1, 'Bank Transfer', 'TRX-9', 'KBank 1', \
            'SLIP-{branch}', 3200.00, 1.50, '{T0}', now(), now())"
    ))
    .await;
    id
}

async fn seed(db: &TestDb) -> Seeded {
    db.branch("B2").await;
    for branch in ["MAIN", "B2"] {
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT 'CU-{branch}', organization_id, 'CU-{branch}', 'Somchai', 'DEMO', now() \
             FROM branches WHERE id = '{branch}'"
        ))
        .await;
        for key in [format!("SLIP-{branch}"), format!("RECEIPT-{branch}")] {
            db.exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('{key}', '{branch}', 'owner', 'application/pdf', 'sha-{key}', now())"
            ))
            .await;
        }
    }
    // a quote the draft was made from (TX-1's table)
    let p = db.publish("MAIN", &[quote_common::USD]).await;
    let quote = uuid::Uuid::new_v4().to_string();
    db.exec(&format!(
        "INSERT INTO quotes (id, organization_id, branch_id, customer_id, type, channel, status, \
            publication_id, expires_at, created_by, created_at, updated_at) \
         VALUES ('{quote}', '{ORG_1}', 'MAIN', 'CU-MAIN', 'INDICATIVE', 'DESK', 'ACTIVE', '{pid}', \
            '{T1}', 'EMP-001', now(), now())",
        pid = p.id
    ))
    .await;

    let q = format!("'{quote}'");
    let draft = exchange(db, "MAIN", "CU-MAIN", "DRAFT", None, &[("quote_id", &q)]).await;

    let request = uuid::Uuid::new_v4().to_string();
    db.exec(&format!(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
            object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
            status, idempotency_key) \
         VALUES ('{request}', '{ORG_1}', 'EXCHANGE_TRANSACTION', 'MAIN', 'obj', 'h', '1', '{{}}', \
            'EMP-001', now(), 'PENDING', 'k-{request}')"
    ))
    .await;
    let r = format!("'{request}'");
    let pending = exchange(
        db,
        "MAIN",
        "CU-MAIN",
        "PENDING_APPROVAL",
        None,
        &[("approval_request_id", &r)],
    )
    .await;

    let t1 = format!("'{T1}'");
    let completed = exchange(
        db,
        "MAIN",
        "CU-MAIN",
        "COMPLETED",
        Some("TEST-XC-20261006-000001"),
        &[("completed_at", &t1)],
    )
    .await;
    db.exec(&format!(
        "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
            document_ref, created_at, updated_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{completed}', 'TEST-XC-20261006-000001', 1, \
            '{T1}', 'RECEIPT-MAIN', now(), now())"
    ))
    .await;

    let cancelled = exchange(
        db,
        "MAIN",
        "CU-MAIN",
        "CANCELLED",
        None,
        &[("cancelled_at", &t1), ("cancel_reason", "'customer left'")],
    )
    .await;

    let other_branch = exchange(db, "B2", "CU-B2", "DRAFT", None, &[]).await;
    Seeded {
        draft,
        pending,
        request,
        completed,
        cancelled,
        other_branch,
        quote,
    }
}

#[tokio::test]
async fn a_draft_answers_its_quote_line_and_payment_with_no_number_and_no_receipt() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let t = transaction(&db, MANAGER, &format!("id={}", s.draft)).await;

    assert_eq!(t["id"], json!(s.draft));
    assert_eq!(t["status"], json!("DRAFT"));
    assert_eq!(
        t["documentRef"],
        Value::Null,
        "D-3: no number before completion"
    );
    assert_eq!(t["branchId"], json!("MAIN"));
    assert_eq!(t["customerId"], json!("CU-MAIN"));
    assert_eq!(t["quoteId"], json!(s.quote));
    assert_eq!(instant(&t["quoteExpiresAt"]), instant(&json!(T1)));
    assert_eq!(
        t["lines"],
        json!([{
            "lineNo": 1,
            "currency": "USD",
            "side": "BUY",
            "quantity": "100.0000",
            "unitSize": "1.0000",
            "rateSnapshot": "32.10000000",
            "thbAmount": "3210.00",
            "feeThb": "10.00",
            "settleThb": "3200.00",
        }]),
        "the line, money and quantities as decimal strings at their stored scale"
    );
    let payment = &t["payment"];
    assert_eq!(
        (
            &payment["method"],
            &payment["reference"],
            &payment["account"],
            &payment["slipFileKey"],
            &payment["amountThb"],
            &payment["bankFeeThb"],
        ),
        (
            &json!("Bank Transfer"),
            &json!("TRX-9"),
            &json!("KBank 1"),
            &json!("SLIP-MAIN"),
            &json!("3200.00"),
            &json!("1.50"),
        ),
        "the payment"
    );
    assert_eq!(instant(&payment["paidAt"]), instant(&json!(T0)));
    assert_eq!(t["receipts"], json!([]), "no receipt before completion");
    assert_eq!(t["approvalRequest"], Value::Null);
    for field in ["completedAt", "cancelledAt", "cancelReason"] {
        assert_eq!(t[field], Value::Null, "{field}");
    }
    db.finish().await;
}

#[tokio::test]
async fn a_pending_exchange_carries_its_approval_request() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let t = transaction(&db, MANAGER, &format!("id={}", s.pending)).await;
    assert_eq!(t["status"], json!("PENDING_APPROVAL"));
    assert_eq!(
        t["approvalRequest"],
        json!({ "id": s.request, "status": "PENDING" }),
        "§10: the desk re-reads this while it waits"
    );
    assert_eq!(t["quoteId"], Value::Null, "this one has no quote");
    assert_eq!(t["quoteExpiresAt"], Value::Null);
    db.finish().await;
}

#[tokio::test]
async fn a_completed_exchange_has_its_number_and_receipt_revision_1() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let t = transaction(&db, MANAGER, &format!("id={}", s.completed)).await;
    assert_eq!(t["status"], json!("COMPLETED"));
    assert_eq!(t["documentRef"], json!("TEST-XC-20261006-000001"));
    assert_eq!(instant(&t["completedAt"]), instant(&json!(T1)));
    let receipts = t["receipts"].as_array().expect("receipts");
    assert_eq!(receipts.len(), 1, "{receipts:?}");
    assert_eq!(
        (&receipts[0]["number"], &receipts[0]["revision"]),
        (&json!("TEST-XC-20261006-000001"), &json!(1))
    );
    assert_eq!(instant(&receipts[0]["issuedAt"]), instant(&json!(T1)));
    db.finish().await;
}

#[tokio::test]
async fn a_cancelled_exchange_says_when_and_why() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let t = transaction(&db, MANAGER, &format!("id={}", s.cancelled)).await;
    assert_eq!(t["status"], json!("CANCELLED"));
    assert_eq!(
        t["documentRef"],
        Value::Null,
        "a cancelled exchange burns no number"
    );
    assert_eq!(instant(&t["cancelledAt"]), instant(&json!(T1)));
    assert_eq!(t["cancelReason"], json!("customer left"));
    db.finish().await;
}

#[tokio::test]
async fn the_branchs_front_and_manager_read_it_and_nobody_signed_in_does() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    for who in ["staff:FRONT:MAIN", "staff:MANAGER:MAIN"] {
        let t = transaction(&db, who, &format!("id={}", s.draft)).await;
        assert_eq!(t["id"], json!(s.draft), "{who}");
    }
    let (status, body) = get(&db, "", &format!("id={}", s.draft)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"]["code"], json!("signIn"), "{body}");
    db.finish().await;
}

#[tokio::test]
async fn another_branchs_unknown_or_malformed_id_answers_null() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    assert_eq!(
        transaction(&db, "staff:FRONT:MAIN", &format!("id={}", s.other_branch)).await,
        Value::Null,
        "a staff caller reads only its own branch"
    );
    assert_eq!(
        transaction(&db, MANAGER, &format!("id={}", s.other_branch)).await,
        Value::Null,
        "MAIN's MANAGER does not read B2's"
    );
    assert_eq!(
        transaction(&db, "staff:MANAGER:B2", &format!("id={}", s.other_branch)).await["id"],
        json!(s.other_branch),
        "B2's MANAGER reads B2's: the null above is the branch scope, not a missing row"
    );
    assert_eq!(
        transaction(&db, "staff:FRONT:B2", &format!("id={}", s.draft)).await,
        Value::Null,
        "B2's FRONT does not read MAIN's"
    );
    let nobody = uuid::Uuid::new_v4();
    assert_eq!(
        transaction(&db, MANAGER, &format!("id={nobody}")).await,
        Value::Null,
        "unknown"
    );
    assert_eq!(
        transaction(&db, MANAGER, "id=not-a-uuid").await,
        Value::Null,
        "malformed"
    );
    db.finish().await;
}

/// Ruling issuecomment-6107277948: a caller holding `adminSnapshot`'s permission (`CompatRead`) but
/// not `TransactionRead` is refused with the code main gives on `EntryPath::Rpc`
/// (`roleNotAllowedHere`), as a client error. The owner (EXECUTIVE + ADMIN), RATES (RATE_MANAGER)
/// and FINANCE all hold `CompatRead` and none holds `TransactionRead`.
#[tokio::test]
async fn a_role_with_admin_snapshots_permission_but_not_transaction_read_is_refused() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    for who in ["owner", "staff:RATES:MAIN", "staff:FINANCE:MAIN"] {
        let (status, body) = get(&db, who, &format!("id={}", s.draft)).await;
        assert!(
            status.is_client_error(),
            "{who}: refused as a client error, got {status}: {body}"
        );
        assert_eq!(
            body["error"]["code"],
            json!("roleNotAllowedHere"),
            "{who}: the policy code main gives on Rpc: {body}"
        );
        assert!(
            body.pointer("/data/transaction").is_none(),
            "{who}: no transaction in a refusal: {body}"
        );
    }
    db.finish().await;
}

/// TELLER (v3 FRONT) and BRANCH_MANAGER (v3 MANAGER and BACKEND) hold `TransactionRead` and read
/// their branch's exchange.
#[tokio::test]
async fn teller_and_branch_manager_read_it() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    for who in [
        "staff:FRONT:MAIN",
        "staff:MANAGER:MAIN",
        "staff:BACKEND:MAIN",
    ] {
        let t = transaction(&db, who, &format!("id={}", s.completed)).await;
        assert_eq!(t["id"], json!(s.completed), "{who}");
        assert_eq!(t["status"], json!("COMPLETED"), "{who}");
    }
    db.finish().await;
}

/// The row (ruling issuecomment-6107277948, `areas/transaction_read.rs`): `transactionView` on
/// `EntryPath::Rpc` needs `TransactionRead`, and nothing wider.
#[test]
fn the_transaction_view_row_needs_transaction_read() {
    assert_eq!(
        Policy::need(EntryPath::Rpc, &Action::new("transactionView")),
        Some(Need::Permission(Permission::TransactionRead))
    );
}
