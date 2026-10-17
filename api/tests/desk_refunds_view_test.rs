//! Ticket #1265 (DK-K) — the desk's list of approved refunds waiting to be posted:
//! `GET /api/v3/views/desk_refunds?branch=`.
//! Written before the implementation by the up-front test writer (claude/07d3765f, on the lane
//! controller's order). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/xchange-v3-wave3-desk.md` §5.5.2 and its behaviour rows RF-1..RF-8 (§5.5.4),
//! merged as `eeea76f4`. [required] there:
//! - `DeskRefundsView`, a `BranchView`: `NAME = "desk_refunds"`, `Filter = NoFilter`,
//!   `INHERITS = (EntryPath::Rpc, "reversalRequest")`, read-only;
//! - `data = DeskRefunds { rows: [DeskRefundRow] }`, camelCase; a row is
//!   `{ reversal: ReversalSummaryDto, receiptNumber, lines: [{ shopSide, currency, quantity, settleThb }],
//!   payments: [{ method, amountThb }] }`, every money and quantity a decimal string;
//! - rows: the branch's reversals with stored `status = 'APPROVED'` and `ReversalWindow::is_open`;
//! - `receiptNumber` is the original's revision-1 receipt; lines by `line_no`, payments by `payment_no`, as stored;
//! - only those fields: no payment `reference`, `account` or slip, no customer.
//!   The order of rows (`approvedAt` ascending) is [proposed] and is not pinned: every test that lists more than
//!   one row compares sets.
//!
//! "As stored" is read literally: a quantity or amount equals its column's text in PostgreSQL
//! (`quantity::text`), so `numeric(20,4)` quantities keep their four places.
//!
//! Rows are seeded with SQL in RV-1's and TX-2's schema, as `v3_reversal_view_test.rs` does. The window is the
//! branch's timezone (Asia/Bangkok): a reversal whose `business_date` is Bangkok's today is inside it, one of
//! Bangkok's yesterday is past it. Sessions come from the `x-test-as` header (`owner`, `staff:<ROLE>:<branch>`).
//! The content tests read as MAIN's FRONT (TELLER: holds `ReversalRequest`).
//!
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod quote_common;

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use quote_common::TestDb;
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
/// Who reads in the content tests: MAIN's FRONT (TELLER, holds `ReversalRequest`).
const ME: &str = "staff:FRONT:MAIN";
const VIEW: &str = "/api/v3/views/desk_refunds";

const COMPLETED_AT: &str = "2026-10-06T05:00:00Z";
const REQUESTED_AT: &str = "2026-10-06T06:00:00Z";
const DECIDED_AT: &str = "2026-10-06T06:30:00Z";
const POSTED_AT: &str = "2026-10-06T07:00:00Z";

/// Bangkok's today, as SQL: inside the window.
const TODAY: &str = "(now() AT TIME ZONE 'Asia/Bangkok')::date";
/// Bangkok's yesterday, as SQL: past the window.
const YESTERDAY: &str = "((now() AT TIME ZONE 'Asia/Bangkok')::date - 1)";

/// Values seeded where the answer must never show them (RF-7).
const SECRETS: [&str; 5] = [
    "REF-SECRET",
    "ACCT-SECRET",
    "SLIP-SECRET",
    "Somchai",
    "CU-SECRET",
];

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

/// `GET desk_refunds?branch=<branch>` as `who`: (status, body, the raw text).
async fn get(db: &TestDb, who: &str, branch: &str) -> (StatusCode, Value, String) {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::get()
        .uri(&format!("{VIEW}?branch={branch}"))
        .insert_header((header::HOST, "xc.test"));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(text.clone()));
    (status, body, text)
}

/// The `data` of a 200 as `who` on `branch`.
async fn data(db: &TestDb, who: &str, branch: &str) -> Value {
    let (status, body, _) = get(db, who, branch).await;
    assert_eq!(status, StatusCode::OK, "{who} {branch}: {body}");
    body.get("data")
        .unwrap_or_else(|| panic!("{who} {branch}: no data in {body}"))
        .clone()
}

fn rows(data: &Value) -> Vec<Value> {
    data["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("data.rows is a list: {data}"))
        .clone()
}

/// The reversal ids listed, as a set (the order is [proposed]).
fn listed(data: &Value) -> BTreeSet<String> {
    rows(data)
        .iter()
        .map(|r| {
            r["reversal"]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("reversal.id: {r}"))
                .to_owned()
        })
        .collect()
}

fn instant(v: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(v.as_str().unwrap_or_else(|| panic!("an instant, got {v}")))
        .unwrap_or_else(|e| panic!("{v} is RFC 3339: {e}"))
        .with_timezone(&Utc)
}

/// The customer, receipt file and slip file every original refers to, once per branch.
async fn prepare(db: &TestDb, branch: &str) {
    if branch != "MAIN" {
        db.branch(branch).await;
    }
    db.exec(&format!(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CU-SECRET-{branch}', organization_id, 'CU-SECRET-{branch}', 'Somchai', 'DEMO', now() \
         FROM branches WHERE id = '{branch}'"
    ))
    .await;
    for key in [format!("RECEIPT-{branch}"), format!("SLIP-SECRET-{branch}")] {
        db.exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('{key}', '{branch}', 'owner', 'text/html', 'sha-{key}', now())"
        ))
        .await;
    }
}

/// A COMPLETED DEMO exchange numbered `number` at `branch`, with two lines and two payments (seeded out of
/// order, so the answer's order is `line_no` / `payment_no`) and its receipt revision 1. The first payment
/// carries a reference, an account and a slip, which the view must not answer. Answers its id.
async fn completed(db: &TestDb, branch: &str, number: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let org = format!("(SELECT organization_id FROM branches WHERE id = '{branch}')");
    db.exec(&format!(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, submission_id, payload_hash, customer_id, \
            customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
            recorded_by, status, completed_at, organization_id) \
         VALUES ('{id}', '{number}', '{branch}', 'DEMO', 'sub-{id}', 'hash', 'CU-SECRET-{branch}', \
            'Somchai', 'THA', 'EMP-001', 'WALK-IN', '', '2026-10-06', '{COMPLETED_AT}', \
            '{COMPLETED_AT}', 'owner', 'COMPLETED', '{COMPLETED_AT}', {org})"
    ))
    .await;
    for (line_no, currency, quantity, settle) in
        [(2, "EUR", "5.5", "200.00"), (1, "USD", "100", "3000.00")]
    {
        db.exec(&format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
                quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, \
                created_at, updated_at) \
             VALUES (gen_random_uuid(), {org}, '{id}', 'BUY', '{currency}', {quantity}, 1, 32.1, \
                {settle}, 0, {settle}, {line_no}, '{COMPLETED_AT}', '{COMPLETED_AT}')"
        ))
        .await;
    }
    for (payment_no, method, reference, account, slip, amount) in [
        (2, "PromptPay", "", "", "NULL".to_owned(), "200.00"),
        (
            1,
            "Cash",
            "REF-SECRET",
            "ACCT-SECRET",
            format!("'SLIP-SECRET-{branch}'"),
            "3000.00",
        ),
    ] {
        db.exec(&format!(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, \
                reference, account, slip_file_key, amount_thb, bank_fee_thb, paid_at, created_at, updated_at) \
             VALUES (gen_random_uuid(), {org}, '{id}', {payment_no}, '{method}', '{reference}', '{account}', \
                {slip}, {amount}, 0, '{COMPLETED_AT}', '{COMPLETED_AT}', '{COMPLETED_AT}')"
        ))
        .await;
    }
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
#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Requested,
    Approved,
    Rejected,
    Posted,
}

/// A reversal of `original` (numbered `number`) at `branch`, at `stage`, with `business_date` (a SQL date
/// expression); answers its id. A POSTED one gets its revision-2 receipt, as RV-3 writes it.
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
    let (posting, receipt, posted_by, posted_at) = if stage == Stage::Posted {
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
            "'EMP-FRONT'".to_owned(),
            format!("'{POSTED_AT}'"),
        )
    } else {
        ("NULL".into(), "NULL".into(), "NULL".into(), "NULL".into())
    };
    db.exec(&format!(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
            reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
            requested_by, requested_at, posted_by, posted_at) \
         VALUES ('{id}', {org}, '{branch}', '{original}', '{status}', 'wrong currency', 3200.00, \
            {business_date}, {request}, {posting}, {receipt}, 'EMP-FRONT', '{REQUESTED_AT}', \
            {posted_by}, {posted_at})"
    ))
    .await;
    id
}

/// One original and its reversal at `branch` in `stage` on `business_date`: (reversal id, original id, number).
async fn one(
    db: &TestDb,
    branch: &str,
    n: u32,
    stage: Stage,
    business_date: &str,
) -> (String, String, String) {
    let number = format!("TEST-XC-{branch}-{n:06}");
    let original = completed(db, branch, &number).await;
    let reversal = reversal_of(db, branch, &original, &number, stage, business_date).await;
    (reversal, original, number)
}

// ── RF-1 ─────────────────────────────────────────────────────────────────────

/// RF-1: an APPROVED reversal inside its window is one row: RV-4's summary (APPROVED, reason, a 2-place amount,
/// `approvedAt` the decision's time), the original's revision-1 receipt number, and its lines and payments as
/// stored, in `line_no` / `payment_no` order.
#[tokio::test]
async fn an_approved_reversal_inside_its_window_is_one_row_with_its_original() {
    let db = TestDb::start().await;
    prepare(&db, "MAIN").await;
    let (id, original, number) = one(&db, "MAIN", 1, Stage::Approved, TODAY).await;

    let d = data(&db, ME, "MAIN").await;
    let all = rows(&d);
    assert_eq!(all.len(), 1, "one row: {d}");
    let row = &all[0];

    let r = &row["reversal"];
    assert_eq!(r["id"], json!(id), "{row}");
    assert_eq!(r["originalTransactionId"], json!(original), "{row}");
    assert_eq!(r["status"], json!("APPROVED"), "{row}");
    assert_eq!(r["reason"], json!("wrong currency"), "{row}");
    assert_eq!(r["amount"], json!("3200.00"), "a 2-place string: {row}");
    assert_eq!(
        instant(&r["approvedAt"]),
        instant(&json!(DECIDED_AT)),
        "{row}"
    );
    assert_eq!(r["postedAt"], Value::Null, "{row}");
    assert_eq!(r["documentNumber"], Value::Null, "{row}");

    assert_eq!(
        row["receiptNumber"],
        json!(number),
        "the original's revision-1 receipt: {row}"
    );

    let stored_lines: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT shop_side, currency, quantity::text, settle_thb::text FROM transaction_lines \
          WHERE transaction_id = $1::uuid ORDER BY line_no",
    )
    .bind(&original)
    .fetch_all(&db.pool)
    .await
    .expect("the stored lines");
    let want_lines: Vec<Value> = stored_lines
        .iter()
        .map(|(side, currency, quantity, settle)| {
            json!({ "shopSide": side, "currency": currency, "quantity": quantity, "settleThb": settle })
        })
        .collect();
    assert_eq!(
        row["lines"],
        json!(want_lines),
        "lines as stored, by line_no: {row}"
    );
    assert_eq!(
        row["lines"][0]["currency"],
        json!("USD"),
        "line_no 1 first: {row}"
    );

    let stored_payments: Vec<(String, String)> = sqlx::query_as(
        "SELECT method, amount_thb::text FROM transaction_payments \
          WHERE transaction_id = $1::uuid ORDER BY payment_no",
    )
    .bind(&original)
    .fetch_all(&db.pool)
    .await
    .expect("the stored payments");
    let want_payments: Vec<Value> = stored_payments
        .iter()
        .map(|(method, amount)| json!({ "method": method, "amountThb": amount }))
        .collect();
    assert_eq!(
        row["payments"],
        json!(want_payments),
        "payments as stored, by payment_no: {row}"
    );
    assert_eq!(
        row["payments"][0]["method"],
        json!("Cash"),
        "payment_no 1 first: {row}"
    );
    db.finish().await;
}

// ── RF-2, RF-3 ───────────────────────────────────────────────────────────────

/// RF-2 and RF-3: REQUESTED, REJECTED and POSTED reversals are never listed, and neither is an APPROVED one past
/// the original's business date (it can never be posted). Two APPROVED ones inside the window are both listed.
#[tokio::test]
async fn only_approved_reversals_inside_their_window_are_listed() {
    let db = TestDb::start().await;
    prepare(&db, "MAIN").await;
    let mut want = BTreeSet::new();
    let mut n = 0;
    for (stage, date, shown) in [
        (Stage::Approved, TODAY, true),
        (Stage::Approved, TODAY, true),
        (Stage::Requested, TODAY, false),
        (Stage::Rejected, TODAY, false),
        (Stage::Posted, TODAY, false),
        (Stage::Approved, YESTERDAY, false),
    ] {
        n += 1;
        let (id, _, _) = one(&db, "MAIN", n, stage, date).await;
        if shown {
            want.insert(id);
        }
    }
    let d = data(&db, ME, "MAIN").await;
    assert_eq!(listed(&d), want, "{d}");
    for row in rows(&d) {
        assert_eq!(row["reversal"]["status"], json!("APPROVED"), "{row}");
    }
    db.finish().await;
}

// ── RF-4 ─────────────────────────────────────────────────────────────────────

/// RF-4: another branch's APPROVED reversal is not listed; each branch's own reader sees only its own.
#[tokio::test]
async fn another_branchs_approved_reversal_is_not_listed() {
    let db = TestDb::start().await;
    prepare(&db, "MAIN").await;
    prepare(&db, "B2").await;
    let (main_id, _, _) = one(&db, "MAIN", 1, Stage::Approved, TODAY).await;
    let (b2_id, _, _) = one(&db, "B2", 2, Stage::Approved, TODAY).await;

    let main = data(&db, ME, "MAIN").await;
    assert_eq!(listed(&main), BTreeSet::from([main_id]), "{main}");
    let b2 = data(&db, "staff:FRONT:B2", "B2").await;
    assert_eq!(listed(&b2), BTreeSet::from([b2_id]), "{b2}");
    db.finish().await;
}

// ── RF-5 ─────────────────────────────────────────────────────────────────────

/// RF-5: a branch with no reversal answers `{ "rows": [] }`.
#[tokio::test]
async fn no_reversal_is_an_empty_list() {
    let db = TestDb::start().await;
    prepare(&db, "MAIN").await;
    completed(&db, "MAIN", "TEST-XC-MAIN-000001").await;
    assert_eq!(data(&db, ME, "MAIN").await, json!({ "rows": [] }));
    db.finish().await;
}

// ── RF-6 ─────────────────────────────────────────────────────────────────────

/// Every caller this file compares: signed out, the owner, and each staff role of MAIN.
fn callers() -> Vec<String> {
    let mut out = vec![String::new(), "owner".to_owned()];
    for role in Role::STAFF {
        out.push(format!("staff:{}:MAIN", role.as_str()));
    }
    out
}

/// RF-6: the view admits exactly whom `Policy::allow` admits for `(Rpc, "reversalRequest")` — the post
/// command's own row — compared caller by caller; a refused caller gets a 4xx refusal with an error code and no
/// data. A signed-out caller is refused. The refusal's code and sentence are the v3 seam's, which reads the
/// caller as an `access::Actor` and so need not word it as `Policy::allow` on the legacy actor does (measured: the
/// owner is `roleNotAllowedHere` here); only who is admitted is pinned.
#[tokio::test]
async fn callers_are_admitted_exactly_as_the_reversal_request_row_admits() {
    let db = TestDb::start().await;
    prepare(&db, "MAIN").await;
    one(&db, "MAIN", 1, Stage::Approved, TODAY).await;
    let action = Action::new("reversalRequest");
    let mut admitted = Vec::new();
    for who in callers() {
        let s = sessions_of(&who);
        let actor = s.owner.or(s.staff);
        let (status, body, text) = get(&db, &who, "MAIN").await;
        let what = format!("as {who:?}");
        match Policy::allow(actor.as_ref(), EntryPath::Rpc, &action) {
            Ok(()) => {
                admitted.push(who.clone());
                assert_eq!(status, StatusCode::OK, "{what}: {text:.300}");
                assert_eq!(rows(&body["data"]).len(), 1, "{what}: {text:.300}");
            }
            Err(_) => {
                assert!(status.is_client_error(), "{what}: refused: {text:.300}");
                assert!(
                    body["error"]["code"].is_string(),
                    "{what}: an error code: {text:.300}"
                );
                assert!(body.get("data").is_none(), "{what}: no data: {text:.300}");
            }
        }
    }
    assert!(
        !admitted.contains(&String::new()),
        "signed out is refused: {admitted:?}"
    );
    assert!(
        admitted.contains(&ME.to_owned()),
        "the teller is admitted: {admitted:?}"
    );
    db.finish().await;
}

// ── RF-7 ─────────────────────────────────────────────────────────────────────

/// Every object key anywhere in `v`, lower-cased.
fn keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                out.push(k.to_lowercase());
                keys(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| keys(x, out)),
        _ => {}
    }
}

/// RF-7: no `reference`, `account`, slip or customer field anywhere in the JSON, and none of their seeded
/// values in the raw answer.
#[tokio::test]
async fn the_answer_has_no_bank_detail_slip_or_customer() {
    let db = TestDb::start().await;
    prepare(&db, "MAIN").await;
    one(&db, "MAIN", 1, Stage::Approved, TODAY).await;
    let (status, body, text) = get(&db, ME, "MAIN").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(rows(&body["data"]).len(), 1, "a row to inspect: {text}");
    let mut all = Vec::new();
    keys(&body, &mut all);
    for key in &all {
        for banned in ["reference", "account", "slip", "customer", "nationality"] {
            assert!(!key.contains(banned), "key {key:?} in {text}");
        }
    }
    for secret in SECRETS {
        assert!(!text.contains(secret), "{secret:?} in {text}");
    }
    db.finish().await;
}

// ── RF-8 ─────────────────────────────────────────────────────────────────────

/// RF-8: reading writes nothing: `reversals`, `ledger_postings`, `receipts` (and `approval_requests`) keep
/// their row counts and the listed reversal stays APPROVED.
#[tokio::test]
async fn reading_writes_nothing() {
    let db = TestDb::start().await;
    prepare(&db, "MAIN").await;
    let (id, _, _) = one(&db, "MAIN", 1, Stage::Approved, TODAY).await;
    let tables = [
        "reversals",
        "ledger_postings",
        "receipts",
        "approval_requests",
    ];
    let mut before = Vec::new();
    for t in tables {
        before.push(db.count(&format!("SELECT count(*) FROM {t}")).await);
    }
    for _ in 0..2 {
        assert_eq!(rows(&data(&db, ME, "MAIN").await).len(), 1);
    }
    let mut after = Vec::new();
    for t in tables {
        after.push(db.count(&format!("SELECT count(*) FROM {t}")).await);
    }
    assert_eq!(after, before, "{tables:?}");
    assert_eq!(
        db.text(&format!("SELECT status FROM reversals WHERE id = '{id}'"))
            .await,
        Some("APPROVED".to_owned())
    );
    db.finish().await;
}

// ── §5.5.2: the view's declared row ──────────────────────────────────────────

/// §5.5.2 [required]: `NAME = "desk_refunds"`, `INHERITS = (EntryPath::Rpc, "reversalRequest")` (the post
/// command's own row) and `VERSIONED = false`. Read from the view's source, because `reversalRequest` and
/// `transactionView` need permissions with the same holders today (TELLER, BRANCH_MANAGER), so no caller can tell
/// the two rows apart through HTTP; a view inheriting the wrong row would pass RF-6 until the holders diverge.
#[test]
fn the_view_declares_the_post_commands_row_and_is_unversioned() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/presentation/http/v3/views/desk_refunds.rs");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} exists: {e}", path.display()));
    let flat: String = source.split_whitespace().collect();
    for want in [
        r#"NAME:&'staticstr="desk_refunds";"#,
        r#"INHERITS:(EntryPath,&'staticstr)=(EntryPath::Rpc,"reversalRequest");"#,
        "VERSIONED:bool=false;",
        "typeFilter=NoFilter;",
    ] {
        assert!(flat.contains(want), "{} declares {want}", path.display());
    }
}
