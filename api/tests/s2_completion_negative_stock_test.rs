//! Ticket #3405 (S2) — a completion short of stock opens a `NEGATIVE_STOCK` approval request, and completes once a
//! BRANCH_MANAGER approves it (design `docs/design/blueprint/ledger-inventory.md` §5.1; rows `section-b-gaps.md`
//! §S2, PR 3387).
//! Written before the implementation by the ticket author (claude/studio-claude-4, on the xchange BE lane controller
//! claude/a90ab16c's order, 2026-10-19). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Decided, and pinned here:
//! - §5.1: the completion of an exchange that would take the drawer below zero opens a request; `may_decide` is the
//!   branch's BRANCH_MANAGER (`NegativeStockPolicy`); APPROVED ⇒ the teller's retry posts with `allow_negative`.
//! - #3405 §7: the answer is 409 `approvalRequired` (already in the TX-5 refusal list, `quote-transaction.md:311`),
//!   the exchange stays CONFIRMED, and the request's `object_id` is the exchange's id, which is what
//!   `LedgerCashPostings::allows_negative` compares with the posting's source (`ledger_postings.rs:889`).
//! - The owner, verbatim on #564 (2026-10-19 08:57Z, relayed by claude/a90ab16c): *"#3405 plain stockInsufficient
//!   after a rejection"*.
//!
//! Not pinned here, on purpose (each waits on a ruling; see #3405):
//! - the request id inside the 409 body: `V3Error` has no slot for it (`api/src/presentation/http/v3/error.rs:9-14`);
//! - the shortfall figures in `proposed_payload` (#3405 open question 2);
//! - whether a NEW `Idempotency-Key` after a rejection opens a new request at once, or the first completion after a
//!   rejection answers `stockInsufficient` whatever its key ("a new request only on the teller's retry");
//! - the creator deciding its own request: only FRONT may `saveTransaction` and FRONT may not `approvalDecide`
//!   (`xc-auth/src/domain/policy/entities/policy.rs:82-86`), so no caller over HTTP is both; LG-3's
//!   `allows_self_decision = false` is covered in `xc-cash/tests/ledger_approvals_test.rs`.
//!
//! The teller is MAIN's FRONT (`TELLER`); the decider is MAIN's `MANAGER` (BRANCH_MANAGER, `access/v3_roles.rs:15`).
//! The exchange is a CONFIRMED LIVE cash SELL of USD 2,000 on MAIN's open shift, whose drawer opens with USD 1,000.00
//! (`tx5_common::Counter`). Everything after the setup goes over the wire, through the real `LedgerCashPostings`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionStatus};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TELLER_AS: &str = "teller";
const MANAGER_AS: &str = "staff:MANAGER:MAIN";

/// Who is signed in, from `x-test-as`: `teller` (FRONT of MAIN, id `TELLER`) or `staff:<ROLE>:<branch>`.
struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    let staff = match spec {
        "" => None,
        "teller" => Actor::staff(TELLER, Role::Front, BranchId::new("MAIN")),
        other => {
            let mut parts = other.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "staff spec {other:?}");
            let role = Role::parse(parts.next().expect("a role")).expect("a known role");
            let branch = parts.next().expect("a branch");
            Actor::staff(
                format!("employee-{}", role.as_str()),
                role,
                BranchId::new(branch),
            )
        }
    };
    Sessions { owner: None, staff }
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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn message(&self) -> &str {
        self.body["error"]["message"].as_str().unwrap_or("")
    }
}

/// `POST path` as `who` with `key` as the `Idempotency-Key`. The receipt files go to the counter's scratch directory.
async fn post(c: &Counter, path: &str, who: &str, body: Value, key: &str) -> Answer {
    let dir = c.scratch.join("uploads");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().expect("a UTF-8 path"),
        )])),
        c.desk.db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::post()
        .uri(path)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header((AS, who))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The teller completes the exchange with `key`.
async fn complete(c: &Counter, t: &ExchangeTransaction, key: &str) -> Answer {
    post(
        c,
        &format!("/api/transactions/{}/complete", t.id.0),
        TELLER_AS,
        json!({}),
        key,
    )
    .await
}

/// MAIN's branch manager decides `request_id` (`APPROVED` or `REJECTED`); the decision must be accepted.
async fn decide(c: &Counter, request_id: &str, outcome: &str) {
    let a = post(
        c,
        &format!("/api/approvals/{request_id}/decide"),
        MANAGER_AS,
        json!({ "outcome": outcome, "reason": "S2 up-front test" }),
        &unique("decide"),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "MAIN's branch manager decides {outcome}: {}",
        a.text
    );
}

/// A CONFIRMED LIVE cash SELL of USD 2,000, beyond the drawer's USD 1,000.00.
async fn short_sell(c: &Counter) -> ExchangeTransaction {
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "2000", None, now).await;
    c.live_of(&q, PaymentMethod::Cash, true, now).await
}

/// The `NEGATIVE_STOCK` requests naming `t`, as `(id, status, branch_id)` in the order they were opened.
async fn requests(c: &Counter, t: &ExchangeTransaction) -> Vec<(String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT id::text, status, branch_id FROM approval_requests \
         WHERE type = 'NEGATIVE_STOCK' AND object_id = $1 ORDER BY requested_at, id",
    )
    .bind(t.id.0.to_string())
    .fetch_all(&c.desk.db.pool)
    .await
    .expect("read approval_requests")
}

/// Every `NEGATIVE_STOCK` request in the schema.
async fn all_negative_stock(c: &Counter) -> i64 {
    c.desk
        .db
        .count("SELECT count(*) FROM approval_requests WHERE type = 'NEGATIVE_STOCK'")
        .await
}

/// The drawer's expected balance of `code` on MAIN's open shift, read from the signed ledger sum.
async fn expected(c: &Counter, code: &str) -> String {
    let sum: rust_decimal::Decimal = sqlx::query_scalar(
        "SELECT COALESCE(SUM(e.quantity_signed), 0) FROM ledger_entries e \
         JOIN cash_accounts a ON a.id = e.account_id \
         WHERE a.kind = 'DRAWER' AND a.shift_id = $1 AND a.currency = $2",
    )
    .bind(&c.shift)
    .bind(code)
    .fetch_one(&c.desk.db.pool)
    .await
    .expect("the drawer's signed ledger sum");
    format!("{sum:.2}")
}

/// The exchange is still CONFIRMED, unnumbered, and the drawer did not move.
async fn assert_not_completed(c: &Counter, t: &ExchangeTransaction) {
    let stored = c.desk.stored(t.id).await;
    assert_eq!(
        stored.status,
        TransactionStatus::Confirmed,
        "still CONFIRMED"
    );
    assert_eq!(stored.document_ref, None, "no number was issued");
    assert_eq!(
        expected(c, "USD").await,
        "1000.00",
        "the drawer's USD did not move"
    );
    assert_eq!(
        expected(c, "THB").await,
        "10000.00",
        "the drawer's THB did not move"
    );
}

// ------------------------------------------------------------ B1: a shortfall opens one PENDING request

#[actix_web::test]
async fn s2_a_sell_beyond_the_drawer_opens_one_pending_request_and_answers_approval_required() {
    let c = Counter::start().await;
    let t = short_sell(&c).await;

    let a = complete(&c, &t, &unique("key")).await;
    assert_eq!(
        (a.status, a.code()),
        (StatusCode::CONFLICT, "approvalRequired"),
        "§5.1: the desk is told to wait for approval: {}",
        a.text
    );
    let rows = requests(&c, &t).await;
    assert_eq!(
        rows.len(),
        1,
        "one NEGATIVE_STOCK request names the exchange: {rows:?}"
    );
    assert_eq!(rows[0].1, "PENDING");
    assert_eq!(
        rows[0].2.as_deref(),
        Some("MAIN"),
        "the request is the teller's branch's, so MAIN's manager may decide it"
    );
    assert_not_completed(&c, &t).await;
    c.finish().await;
}

// ------------------------------------------------------------ B2: no second request while one is pending

#[actix_web::test]
async fn s2_the_same_key_again_opens_no_second_request() {
    let c = Counter::start().await;
    let t = short_sell(&c).await;
    let key = unique("key");

    let first = complete(&c, &t, &key).await;
    assert_eq!(first.code(), "approvalRequired", "{}", first.text);
    let again = complete(&c, &t, &key).await;
    assert_eq!(
        (again.status, again.code()),
        (StatusCode::CONFLICT, "approvalRequired"),
        "{}",
        again.text
    );
    assert_eq!(requests(&c, &t).await.len(), 1, "still one request");
    assert_not_completed(&c, &t).await;
    c.finish().await;
}

#[actix_web::test]
async fn s2_a_new_key_while_a_request_is_pending_opens_no_second_request() {
    let c = Counter::start().await;
    let t = short_sell(&c).await;

    let first = complete(&c, &t, &unique("key")).await;
    assert_eq!(first.code(), "approvalRequired", "{}", first.text);
    let other = complete(&c, &t, &unique("key")).await;
    assert_eq!(
        (other.status, other.code()),
        (StatusCode::CONFLICT, "approvalRequired"),
        "#3405 §7: AlreadyPending answers the same as the first: {}",
        other.text
    );
    let rows = requests(&c, &t).await;
    assert_eq!(rows.len(), 1, "still one request: {rows:?}");
    assert_eq!(rows[0].1, "PENDING");
    assert_not_completed(&c, &t).await;
    c.finish().await;
}

// ------------------------------------------------------------ B3: APPROVED ⇒ the retry completes below zero

#[actix_web::test]
async fn s2_once_the_branch_manager_approves_the_retry_completes_and_the_drawer_goes_below_zero() {
    let c = Counter::start().await;
    let t = short_sell(&c).await;

    let first = complete(&c, &t, &unique("key")).await;
    assert_eq!(first.code(), "approvalRequired", "{}", first.text);
    let rows = requests(&c, &t).await;
    assert_eq!(rows.len(), 1, "one request to decide: {rows:?}");
    decide(&c, &rows[0].0, "APPROVED").await;

    let retry = complete(&c, &t, &unique("key")).await;
    assert_eq!(
        retry.status,
        StatusCode::OK,
        "§5.1: the retry posts with allow_negative: {}",
        retry.text
    );
    assert_eq!(retry.body["status"], "COMPLETED", "{}", retry.text);
    let stored = c.desk.stored(t.id).await;
    assert_eq!(stored.status, TransactionStatus::Completed);
    assert!(stored.document_ref.is_some(), "a number was issued");
    assert_eq!(
        expected(&c, "USD").await,
        "-1000.00",
        "§4.5: the drawer may go negative once approved (1,000.00 − 2,000.00)"
    );
    assert_eq!(
        requests(&c, &t).await.len(),
        1,
        "the retry opened no other request"
    );
    c.finish().await;
}

// ------------------------------------------------------------ the owner's answer: after a rejection

#[actix_web::test]
async fn s2_after_a_rejection_the_teller_gets_stock_insufficient_noting_it_and_no_new_request() {
    let c = Counter::start().await;
    let t = short_sell(&c).await;
    let key = unique("key");

    let first = complete(&c, &t, &key).await;
    assert_eq!(first.code(), "approvalRequired", "{}", first.text);
    let rows = requests(&c, &t).await;
    assert_eq!(rows.len(), 1, "one request to decide: {rows:?}");
    decide(&c, &rows[0].0, "REJECTED").await;

    let after = complete(&c, &t, &key).await;
    assert_eq!(
        (after.status, after.code()),
        (StatusCode::CONFLICT, "stockInsufficient"),
        "the owner: plain stockInsufficient after a rejection: {}",
        after.text
    );
    assert!(
        after.message().to_lowercase().contains("reject"),
        "the owner: the answer notes the rejection: {:?}",
        after.message()
    );
    let rows = requests(&c, &t).await;
    assert_eq!(rows.len(), 1, "no new request: {rows:?}");
    assert_eq!(
        rows[0].1, "REJECTED",
        "the rejected request is never reused or reopened"
    );
    assert_not_completed(&c, &t).await;
    c.finish().await;
}

// ------------------------------------------------------------ B5: enough stock asks nobody (guard)

#[actix_web::test]
async fn s2_a_completion_within_the_drawers_stock_opens_no_request() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Sell, PaymentMethod::Cash, true, now).await;

    let a = complete(&c, &t, &unique("key")).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "USD 100 is within the drawer: {}",
        a.text
    );
    assert_eq!(all_negative_stock(&c).await, 0, "no request is opened");
    c.finish().await;
}
