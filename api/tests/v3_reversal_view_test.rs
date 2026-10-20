//! Ticket #685 (RV-4) — the reads of a reversal: `GET /api/v3/views/reversal?id=` and the
//! `reversal` field of TX-10's B04 view (`GET /api/v3/views/transaction?id=`).
//! Written before the implementation by the ticket author (claude/07d3765f, on the lane
//! controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/reversal.md` §4.1 (the states; "expired" is derived), §4.4 (the
//! reads). The lane controller's ruling on #685 (issuecomment-6097970564), [required]:
//! - **one DTO for both reads**, `ReversalSummaryDto`, camelCase: `{ id, originalTransactionId,
//!   status, reason, amount, requestedBy, requestedAt, approvedAt, postedAt, documentNumber }`,
//!   money as a 2-place string, instants RFC 3339 (compared here as instants);
//! - the view `reversal` (filter `{ id }`) answers `data = { reversal: ReversalSummaryDto | null }`;
//!   `TransactionDetailDto.reversal` is `ReversalSummaryDto | null`;
//! - `approvedAt` is the APPROVED decision's time (AP-1's `approval_decisions.at`): null while
//!   REQUESTED and for a REJECTED reversal;
//! - `status` is one of the four stored states, or `EXPIRED`: derived when the stored status is
//!   APPROVED and the window (R-1, `ReversalWindow`) has closed. The row stays APPROVED;
//! - `documentNumber` is the reversal's receipt (revision 2, `kind REVERSAL`), null until POSTED;
//! - the readers are TX-10's. Under #678's ruling A (issuecomment-6107277948) the B04 view needs `TransactionRead`,
//!   held by TELLER (v3 FRONT) and BRANCH_MANAGER (v3 MANAGER): the branch's FRONT and MANAGER read, and the owner
//!   (EXECUTIVE + ADMIN, no `TransactionRead`) is refused with `roleNotAllowedHere`. The reversal view has the same
//!   readers (#685 Q4), so it reaches `TransactionRead` the same way;
//! - an id that is unknown, malformed or of another branch answers `null`, as TX-10 does.
//!
//! Rows are seeded with SQL in RV-1's and TX-2's schema; a POSTED reversal is seeded as RV-3 will
//! write it (posting and document), because posting is RV-3's. Exchanges are DEMO (no shift). The
//! window is MAIN's timezone, Asia/Bangkok: a reversal whose `business_date` is Bangkok's today is
//! inside it, one of 2026-10-06 is past it. Sessions come from the `x-test-as` header (`owner`,
//! `staff:<ROLE>:<branch>`), as in `v3_transaction_view_test.rs`. The content tests read as MAIN's MANAGER
//! (`ME`), as #678's do under ruling A.
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use quote_common::TestDb;
use serde_json::{Value, json};
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
/// Who reads in the content tests: MAIN's MANAGER (BRANCH_MANAGER, holds `TransactionRead`; #678 ruling A).
const ME: &str = "staff:MANAGER:MAIN";

struct HeaderCallers;

fn roles_of(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown role {other}"),
    }
}

fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return test_callers::nobody();
    }
    if spec == "owner" {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "spec {spec:?}");
    let role = parts.next().expect("a role");
    let roles = roles_of(role);
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

/// `GET <path>?<query>` as `who`: (status, body).
async fn get(db: &TestDb, who: &str, path: &str, query: &str) -> (StatusCode, Value) {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::get()
        .uri(&format!("{path}?{query}"))
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

/// The `reversal` the reversal view answers (`null` included); panics unless 200.
async fn reversal(db: &TestDb, who: &str, id: &str) -> Value {
    let (status, body) = get(db, who, "/api/v3/views/reversal", &format!("id={id}")).await;
    assert_eq!(status, StatusCode::OK, "{who} {id}: {body}");
    body.pointer("/data/reversal")
        .unwrap_or_else(|| panic!("{who} {id}: no data.reversal in {body}"))
        .clone()
}

/// The `reversal` field of the B04 transaction view for the exchange `id`; panics unless 200.
async fn b04_reversal(db: &TestDb, who: &str, id: &str) -> Value {
    let (status, body) = get(db, who, "/api/v3/views/transaction", &format!("id={id}")).await;
    assert_eq!(status, StatusCode::OK, "{who} {id}: {body}");
    let t = body
        .pointer("/data/transaction")
        .unwrap_or_else(|| panic!("{who} {id}: no data.transaction in {body}"));
    assert!(
        t.is_object(),
        "{who} {id}: the transaction is found: {body}"
    );
    t.as_object()
        .expect("an object")
        .get("reversal")
        .unwrap_or_else(|| panic!("{who} {id}: TransactionDetailDto has no `reversal`: {t}"))
        .clone()
}

fn instant(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(v.as_str().unwrap_or_else(|| panic!("an instant, got {v}")))
        .unwrap_or_else(|e| panic!("{v} is RFC 3339: {e}"))
        .with_timezone(&Utc)
}

const COMPLETED_AT: &str = "2026-10-06T05:00:00Z";
const REQUESTED_AT: &str = "2026-10-06T06:00:00Z";
const DECIDED_AT: &str = "2026-10-06T06:30:00Z";
const POSTED_AT: &str = "2026-10-06T07:00:00Z";

/// A COMPLETED DEMO BUY of 100 USD (the shop paid 3200.00) numbered `number` at `branch`, with its
/// receipt revision 1; answers its id.
async fn completed(db: &TestDb, branch: &str, number: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let org = format!("(SELECT organization_id FROM branches WHERE id = '{branch}')");
    db.exec(&format!(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, submission_id, payload_hash, customer_id, \
            customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
            recorded_by, status, completed_at, organization_id) \
         VALUES ('{id}', '{number}', '{branch}', 'DEMO', 'sub-{id}', 'hash', 'CU-{branch}', 'Somchai', \
            'THA', 'EMP-001', 'WALK-IN', '', '2026-10-06', '{COMPLETED_AT}', '{COMPLETED_AT}', \
            'owner', 'COMPLETED', '{COMPLETED_AT}', {org})"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
            quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, \
            created_at, updated_at) \
         VALUES (gen_random_uuid(), {org}, '{id}', 'BUY', 'USD', 100, 1, 32.1, 3210.00, 10.00, \
            3200.00, 1, '{COMPLETED_AT}', '{COMPLETED_AT}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
            reference, account, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
         VALUES (gen_random_uuid(), {org}, '{id}', 1, 'Cash', '', 'drawer', 3200.00, 0, \
            '{COMPLETED_AT}', '{COMPLETED_AT}', '{COMPLETED_AT}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
            document_ref, kind, created_at, updated_at) \
         VALUES (gen_random_uuid(), {org}, '{id}', '{number}', 1, '{COMPLETED_AT}', \
            'RECEIPT-{branch}', 'RECEIPT', '{COMPLETED_AT}', '{COMPLETED_AT}')"
    ))
    .await;
    id
}

/// How far a seeded reversal got.
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Requested,
    Approved,
    Rejected,
    Posted,
}

/// A reversal of `original` (numbered `number`) at `branch`, at `stage`, with `business_date`
/// (a SQL date expression); answers its id.
async fn reversal_of(
    db: &TestDb,
    branch: &str,
    original: &str,
    number: &str,
    stage: Stage,
    business_date: &str,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let org = format!("(SELECT organization_id FROM branches WHERE id = '{branch}')");
    let request = if stage == Stage::Requested {
        "NULL".to_owned()
    } else {
        let request = uuid::Uuid::new_v4().to_string();
        let status = if stage == Stage::Rejected {
            "REJECTED"
        } else {
            "APPROVED"
        };
        db.exec(&format!(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
                status, idempotency_key) \
             VALUES ('{request}', {org}, 'TRANSACTION_REVERSAL', '{branch}', '{id}', 'h', '1', \
                '{{}}', 'EMP-FRONT', '{REQUESTED_AT}', '{status}', 'k-{request}')"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO approval_decisions (id, request_id, approver_id, approver_role, outcome, \
                reason, at) \
             VALUES (gen_random_uuid(), '{request}', 'EMP-FINANCE', 'FINANCE', '{status}', \
                'checked', '{DECIDED_AT}')"
        ))
        .await;
        format!("'{request}'")
    };
    let status = match stage {
        Stage::Requested => "REQUESTED",
        Stage::Approved => "APPROVED",
        Stage::Rejected => "REJECTED",
        Stage::Posted => "POSTED",
    };
    let (posting, receipt, posted_at) = if stage == Stage::Posted {
        let receipt = uuid::Uuid::new_v4().to_string();
        db.exec(&format!(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, \
                issued_at, document_ref, kind, created_at, updated_at) \
             VALUES ('{receipt}', {org}, '{original}', '{number}-R2', 2, '{POSTED_AT}', \
                'RECEIPT-{branch}', 'REVERSAL', '{POSTED_AT}', '{POSTED_AT}')"
        ))
        .await;
        let posting_id = uuid::Uuid::new_v4().to_string();
        db.exec(&format!(
            "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
             VALUES ('{posting_id}', {org}, '{branch}', 'REVERSAL', '{id}', 'reversal', 'POSTED', '{POSTED_AT}', 'EMP-FRONT')"
        ))
        .await;
        (
            format!("'{posting_id}'"),
            format!("'{receipt}'"),
            format!("'{POSTED_AT}'"),
        )
    } else {
        ("NULL".into(), "NULL".into(), "NULL".into())
    };
    db.exec(&format!(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
            reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
            requested_by, requested_at, posted_by, posted_at) \
         VALUES ('{id}', {org}, '{branch}', '{original}', '{status}', 'wrong currency', 3200.00, \
            {business_date}, {request}, {posting}, {receipt}, 'EMP-FRONT', '{REQUESTED_AT}', \
            {posted_by}, {posted_at})",
        posted_by = if stage == Stage::Posted {
            "'EMP-FRONT'"
        } else {
            "NULL"
        },
    ))
    .await;
    id
}

/// Bangkok's today, as SQL: inside the window.
const TODAY: &str = "(now() AT TIME ZONE 'Asia/Bangkok')::date";
/// The original's business date, long past: outside the window.
const PAST: &str = "DATE '2026-10-06'";

struct Seeded {
    posted: (String, String),
    rejected: (String, String),
    expired: (String, String),
    approved: (String, String),
    requested: (String, String),
    other_branch: (String, String),
    unreversed: String,
}

async fn seed(db: &TestDb) -> Seeded {
    db.branch("B2").await;
    for branch in ["MAIN", "B2"] {
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, \
                created_at) \
             SELECT 'CU-{branch}', organization_id, 'CU-{branch}', 'Somchai', 'DEMO', now() \
             FROM branches WHERE id = '{branch}'"
        ))
        .await;
        db.exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('RECEIPT-{branch}', '{branch}', 'owner', 'text/html', 'sha-{branch}', now())"
        ))
        .await;
    }
    let mut made = Vec::new();
    for (n, (branch, stage, date)) in [
        ("MAIN", Stage::Posted, PAST),
        ("MAIN", Stage::Rejected, PAST),
        ("MAIN", Stage::Approved, PAST),
        ("MAIN", Stage::Approved, TODAY),
        ("MAIN", Stage::Requested, TODAY),
        ("B2", Stage::Posted, PAST),
    ]
    .into_iter()
    .enumerate()
    {
        let number = format!("TEST-XC-20261006-{:06}", n + 1);
        let original = completed(db, branch, &number).await;
        let reversal = reversal_of(db, branch, &original, &number, stage, date).await;
        made.push((reversal, original));
    }
    let unreversed = completed(db, "MAIN", "TEST-XC-20261006-000099").await;
    let mut made = made.into_iter();
    Seeded {
        posted: made.next().unwrap(),
        rejected: made.next().unwrap(),
        expired: made.next().unwrap(),
        approved: made.next().unwrap(),
        requested: made.next().unwrap(),
        other_branch: made.next().unwrap(),
        unreversed,
    }
}

#[tokio::test]
async fn a_posted_reversal_shows_its_status_reason_and_document_number() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let (id, original) = &s.posted;
    let r = reversal(&db, ME, id).await;

    assert_eq!(r["id"], json!(id));
    assert_eq!(r["originalTransactionId"], json!(original));
    assert_eq!(r["status"], json!("POSTED"));
    assert_eq!(r["reason"], json!("wrong currency"));
    assert_eq!(r["amount"], json!("3200.00"), "the settled THB, 2 places");
    assert_eq!(r["requestedBy"], json!("EMP-FRONT"));
    assert_eq!(instant(&r["requestedAt"]), instant(&json!(REQUESTED_AT)));
    assert_eq!(
        instant(&r["approvedAt"]),
        instant(&json!(DECIDED_AT)),
        "the APPROVED decision's time"
    );
    assert_eq!(instant(&r["postedAt"]), instant(&json!(POSTED_AT)));
    assert_eq!(
        r["documentNumber"],
        json!("TEST-XC-20261006-000001-R2"),
        "the reversal's receipt, revision 2"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_b04_field_is_the_same_summary_as_the_view() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    for (what, (id, original)) in [
        ("posted", &s.posted),
        ("rejected", &s.rejected),
        ("expired", &s.expired),
        ("approved", &s.approved),
        ("requested", &s.requested),
    ] {
        let from_view = reversal(&db, ME, id).await;
        assert!(from_view.is_object(), "{what}: found by the view");
        assert_eq!(
            b04_reversal(&db, ME, original).await,
            from_view,
            "{what}: one DTO for both reads"
        );
    }
    assert_eq!(
        b04_reversal(&db, ME, &s.unreversed).await,
        Value::Null,
        "an exchange with no reversal"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_rejected_reversal_shows_as_rejected_with_no_approval_or_document() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let r = reversal(&db, ME, &s.rejected.0).await;
    assert_eq!(r["status"], json!("REJECTED"));
    assert_eq!(r["reason"], json!("wrong currency"));
    for field in ["approvedAt", "postedAt", "documentNumber"] {
        assert_eq!(r[field], Value::Null, "{field}");
    }
    db.finish().await;
}

#[tokio::test]
async fn an_approved_reversal_past_its_window_shows_as_expired_and_inside_it_as_approved() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let expired = reversal(&db, ME, &s.expired.0).await;
    assert_eq!(expired["status"], json!("EXPIRED"), "derived (§4.1)");
    assert_eq!(instant(&expired["approvedAt"]), instant(&json!(DECIDED_AT)));
    assert_eq!(expired["documentNumber"], Value::Null);
    let stored: String = sqlx::query_scalar("SELECT status FROM reversals WHERE id = $1::uuid")
        .bind(&s.expired.0)
        .fetch_one(&db.pool)
        .await
        .expect("the row");
    assert_eq!(stored, "APPROVED", "the stored state does not change");

    let approved = reversal(&db, ME, &s.approved.0).await;
    assert_eq!(approved["status"], json!("APPROVED"), "inside the window");
    assert_eq!(
        instant(&approved["approvedAt"]),
        instant(&json!(DECIDED_AT))
    );
    db.finish().await;
}

#[tokio::test]
async fn a_requested_reversal_has_no_approval_yet() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let r = reversal(&db, ME, &s.requested.0).await;
    assert_eq!(r["status"], json!("REQUESTED"));
    for field in ["approvedAt", "postedAt", "documentNumber"] {
        assert_eq!(r[field], Value::Null, "{field}");
    }
    db.finish().await;
}

#[tokio::test]
async fn another_branchs_unknown_or_malformed_id_answers_null() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let (b2, b2_original) = &s.other_branch;
    assert_eq!(
        reversal(&db, ME, b2).await,
        Value::Null,
        "MAIN's manager never learns of B2's reversal"
    );
    assert_eq!(
        reversal(&db, "staff:FRONT:MAIN", b2).await,
        Value::Null,
        "MAIN's staff never learn of B2's reversal"
    );
    assert_eq!(
        reversal(&db, "staff:FRONT:B2", b2).await["documentNumber"],
        json!("TEST-XC-20261006-000006-R2"),
        "B2's own staff read it"
    );
    assert_eq!(
        b04_reversal(&db, "staff:FRONT:B2", b2_original).await["id"],
        json!(b2)
    );
    assert_eq!(
        reversal(&db, "staff:MANAGER:B2", b2).await["documentNumber"],
        json!("TEST-XC-20261006-000006-R2"),
        "B2's manager reads B2's, not MAIN's"
    );
    assert_eq!(
        reversal(&db, "staff:MANAGER:B2", &s.posted.0).await,
        Value::Null
    );
    let nobody = uuid::Uuid::new_v4().to_string();
    assert_eq!(reversal(&db, ME, &nobody).await, Value::Null, "unknown");
    assert_eq!(
        reversal(&db, ME, "not-a-uuid").await,
        Value::Null,
        "malformed"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_branchs_front_and_manager_read_it_and_nobody_signed_in_does() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    for who in ["staff:FRONT:MAIN", "staff:MANAGER:MAIN"] {
        assert_eq!(
            reversal(&db, who, &s.posted.0).await["status"],
            json!("POSTED"),
            "{who}"
        );
    }
    let (status, body) = get(
        &db,
        "",
        "/api/v3/views/reversal",
        &format!("id={}", s.posted.0),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    db.finish().await;
}

/// #678's ruling A under §11.2: `SYSTEM` (`ADMIN`) holds `adminSnapshot`'s permission (`CompatRead`) but not
/// `TransactionRead`, so both reads refuse it with the policy code main gives on Rpc, and answer no reversal.
#[tokio::test]
async fn a_system_caller_without_transaction_read_is_refused_by_both_reads() {
    let db = TestDb::start().await;
    let s = seed(&db).await;
    let (id, original) = &s.posted;
    for (path, query) in [
        ("/api/v3/views/reversal", format!("id={id}")),
        ("/api/v3/views/transaction", format!("id={original}")),
    ] {
        let (status, body) = get(&db, "staff:SYSTEM:MAIN", path, &query).await;
        assert!(
            status.is_client_error(),
            "{path}: refused as a client error, got {status}: {body}"
        );
        assert_eq!(
            body["error"]["code"],
            json!("roleNotAllowedHere"),
            "{path}: {body}"
        );
        assert!(
            body.pointer("/data").is_none(),
            "{path}: no data in a refusal: {body}"
        );
    }
    db.finish().await;
}
